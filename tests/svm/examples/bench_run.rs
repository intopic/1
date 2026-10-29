//! Validator benchmark, step 2 (acceptance A-10): sends the same number of
//! independent swaps to a local `solana-test-validator` spread over 1, 2, 4
//! and 8 shards, and measures how many land per slot. Each configuration is
//! its own market with the same total liquidity, split over its lanes.
//!
//! Every trader has its own fee payer and token accounts, so the only
//! writable accounts two swaps can share are the shard's (shard state, vaults,
//! tick arrays). With 1 shard every swap contends on the same accounts; with
//! 8 shards they split into 8 independent lanes. Each swap requests 60k CU
//! (`SetComputeUnitLimit`), as a router would. For the busiest slot of each
//! run the tool also reads the block and sums the compute units the swaps
//! actually used, so the per-slot ceiling can be compared with the validator's
//! block and per-account cost limits.
//!
//! This measures a single-node test validator on one machine. It does not
//! measure mainnet, gossip, leader rotation or priority-fee markets.
//!
//! Usage: cargo run --release --example bench_run -- <bench-dir> <rpc-url> \
//!        [rounds] [repeats] [out.md]

#[path = "../tests/common/mod.rs"]
mod common;
#[path = "shared/rpc.rs"]
mod rpc;

use anchor_lang::{InstructionData, ToAccountMetas};
use base64::Engine;
use common::{with_remaining, Pubkey, TOKEN_PROGRAM};
use rpc::Rpc;
use serde_json::{json, Value};
use sharded_amm::instructions::SwapArgs;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::{Hash, Message, VersionedMessage};
use solana_signer::Signer;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const COMPUTE_BUDGET: Pubkey = anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111");
const CU_LIMIT: u32 = 60_000;
const SENDERS: usize = 16;

fn key(v: &Value) -> Pubkey {
    Pubkey::from_str(v.as_str().unwrap()).unwrap()
}

struct Trader {
    kp: Keypair,
    token_a: Pubkey,
    token_b: Pubkey,
}

struct Shard {
    shard: Pubkey,
    vault_a: Pubkey,
    vault_b: Pubkey,
    vault_authority: Pubkey,
    arrays_a_to_b: Vec<Pubkey>,
    arrays_b_to_a: Vec<Pubkey>,
    market: Pubkey,
    registry: Pubkey,
}

/// One swap by `t`; `payer` pays the fee (the trader itself, or a shared
/// payer for the negative control).
fn swap_tx(
    s: &Shard,
    t: &Trader,
    payer: &Keypair,
    a_to_b: bool,
    amount: u64,
    blockhash: &Hash,
) -> (String, String) {
    let (tin, tout) = if a_to_b {
        (t.token_a, t.token_b)
    } else {
        (t.token_b, t.token_a)
    };
    let accounts = sharded_amm::accounts::SwapExactIn {
        user: t.kp.pubkey(),
        market: s.market,
        registry: s.registry,
        shard: s.shard,
        vault_a: s.vault_a,
        vault_b: s.vault_b,
        user_token_in: tin,
        user_token_out: tout,
        vault_authority: s.vault_authority,
        token_program: TOKEN_PROGRAM,
    };
    let args = SwapArgs {
        a_to_b,
        amount_in: amount,
        min_amount_out: 0,
        sqrt_price_limit_x64: 0,
        max_rounding_dust: u64::MAX,
        deadline: i64::MAX,
    };
    let swap = with_remaining(
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::SwapExactIn { args }.data(),
        },
        if a_to_b {
            &s.arrays_a_to_b
        } else {
            &s.arrays_b_to_a
        },
    );
    let mut cu = vec![2u8];
    cu.extend_from_slice(&CU_LIMIT.to_le_bytes());
    let budget = Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data: cu,
    };
    let msg = VersionedMessage::Legacy(Message::new_with_blockhash(
        &[budget, swap],
        Some(&payer.pubkey()),
        blockhash,
    ));
    let body = msg.serialize();
    // The fee payer signs first; the trader co-signs when it is not the payer.
    let sig = payer.sign_message(&body);
    let mut signers = vec![sig];
    if payer.pubkey() != t.kp.pubkey() {
        signers.push(t.kp.sign_message(&body));
    }
    let mut wire = vec![signers.len() as u8];
    for s in &signers {
        wire.extend_from_slice(s.as_ref());
    }
    wire.extend_from_slice(&body);
    (
        sig.to_string(),
        base64::engine::general_purpose::STANDARD.encode(wire),
    )
}

/// How each swap's shard is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Trader `i` uses lane `i % n` (an ideal spread).
    RoundRobin,
    /// `amm_router::plan` with power of two choices, from one shared snapshot.
    Router,
    /// `amm_router::plan` always taking the global best (`choices = 0`).
    RouterBest,
    /// Orders of realistic size, planned by `amm_router::plan` with this
    /// split tax (bps), each one transaction over all its legs (v0 + ALT).
    Sized { tax: u32 },
}

#[derive(Clone, Debug)]
struct Outcome {
    shards: usize,
    mode: Mode,
    /// Negative control: every swap has the same fee payer.
    shared_payer: bool,
    sent: usize,
    ok: usize,
    failed: usize,
    missing: usize,
    max_per_slot: usize,
    /// Compute units consumed by all transactions in the busiest slot.
    busiest_cu: u64,
    /// Slots from the start of sending until landing, p50 and p95.
    lat_p50: u64,
    lat_p95: u64,
    send_ms: u128,
    /// Shards per order (1 except in `Mode::Sized`).
    legs: f64,
    /// The first few transaction errors, for the log (read through Debug).
    #[allow(dead_code)]
    errors: Vec<String>,
}

/// One measured burst: `rounds` swaps per trader over the first `n_shards`.
struct Load {
    n_shards: usize,
    rounds: usize,
    /// Makes every amount, and so every signature, unique across bursts.
    salt: u64,
    /// Negative control: one fee payer for every swap.
    shared_payer: bool,
    mode: Mode,
    /// For router modes: lane of every `[round][trader]` swap.
    pick: Option<Arc<Vec<Vec<usize>>>>,
}

/// `shards` are the lanes of the market built for `load.n_shards`.
fn run(rpc: &Arc<Rpc>, shards: &[Shard], traders: &[Trader], load: Load) -> Outcome {
    let Load {
        n_shards,
        rounds,
        salt,
        shared_payer,
        mode,
        pick,
    } = load;
    let blockhash = rpc.blockhash();
    let mut txs = Vec::new();
    for round in 0..rounds {
        for (i, t) in traders.iter().enumerate() {
            let s = match &pick {
                Some(p) => &shards[p[round][i]],
                None => &shards[i % n_shards],
            };
            let a_to_b = (i + round) % 2 == 0;
            // Distinct amounts keep every signature unique across runs.
            let amount = 10_000 + salt * 1_000 + round as u64;
            let payer = if shared_payer { &traders[0].kp } else { &t.kp };
            txs.push(swap_tx(s, t, payer, a_to_b, amount, &blockhash));
        }
    }
    measure(rpc, txs, n_shards, mode, shared_payer)
}

/// Sends `txs` from `SENDERS` threads and measures how they landed.
fn measure(
    rpc: &Arc<Rpc>,
    txs: Vec<(String, String)>,
    n_shards: usize,
    mode: Mode,
    shared_payer: bool,
) -> Outcome {
    let sigs: Vec<String> = txs.iter().map(|t| t.0.clone()).collect();
    let queue = Arc::new(Mutex::new(txs.into_iter().map(|t| t.1).collect::<Vec<_>>()));
    let start_slot = rpc.slot();
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..SENDERS {
        let (rpc, queue) = (rpc.clone(), queue.clone());
        handles.push(std::thread::spawn(move || loop {
            let Some(tx) = queue.lock().unwrap().pop() else {
                break;
            };
            rpc.call(
                "sendTransaction",
                json!([tx, {"encoding": "base64", "skipPreflight": true, "maxRetries": 0}]),
            );
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let send_ms = start.elapsed().as_millis();

    // Wait until every signature has a status, or 40 s pass.
    let mut status: Vec<Option<(u64, bool)>> = vec![None; sigs.len()];
    let mut errors: Vec<String> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(40);
    while status.iter().any(Option::is_none) && Instant::now() < deadline {
        for (c, chunk) in sigs.chunks(256).enumerate() {
            let r = rpc.call(
                "getSignatureStatuses",
                json!([chunk, {"searchTransactionHistory": true}]),
            );
            for (j, v) in r["value"].as_array().unwrap().iter().enumerate() {
                if v.is_null() {
                    continue;
                }
                let done = v["confirmationStatus"] == "confirmed"
                    || v["confirmationStatus"] == "finalized";
                if done {
                    status[c * 256 + j] = Some((v["slot"].as_u64().unwrap(), v["err"].is_null()));
                    if !v["err"].is_null() && errors.len() < 3 {
                        errors.push(v["err"].to_string());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    let ok_slots: Vec<u64> = status
        .iter()
        .flatten()
        .filter(|s| s.1)
        .map(|s| s.0)
        .collect();
    let failed = status.iter().flatten().filter(|s| !s.1).count();
    let missing = status.iter().filter(|s| s.is_none()).count();
    let mut per_slot = std::collections::HashMap::new();
    for s in &ok_slots {
        *per_slot.entry(*s).or_insert(0usize) += 1;
    }
    let mut lat: Vec<u64> = ok_slots
        .iter()
        .map(|s| s.saturating_sub(start_slot))
        .collect();
    lat.sort_unstable();
    let pct = |p: usize| {
        lat.get((lat.len() * p / 100).min(lat.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    };
    let (lat_p50, lat_p95) = (pct(50), pct(95));
    let busiest = per_slot.iter().max_by_key(|e| *e.1).map(|e| *e.0);
    let busiest_cu = busiest.map_or(0, |slot| {
        let b = rpc.call(
            "getBlock",
            json!([slot, {"transactionDetails": "full", "maxSupportedTransactionVersion": 0, "rewards": false, "commitment": "confirmed"}]),
        );
        b["transactions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["meta"]["computeUnitsConsumed"].as_u64().unwrap_or(0))
            .sum()
    });
    Outcome {
        busiest_cu,
        lat_p50,
        lat_p95,
        shared_payer,
        shards: n_shards,
        mode,
        sent: sigs.len(),
        ok: ok_slots.len(),
        failed,
        missing,
        max_per_slot: per_slot.values().copied().max().unwrap_or(0),
        send_ms,
        legs: 1.0,
        errors,
    }
}

/// Size of every `[round][trader]` order as a multiple of the median: a
/// log-normal draw (sigma 1, as in the fee simulation) shared by rounds
/// `2k` and `2k + 1`, which trade in opposite directions, so prices drift
/// little over a burst.
fn size_factors(rounds: usize, traders: usize) -> Vec<Vec<f64>> {
    (0..rounds)
        .map(|r| {
            (0..traders)
                .map(|i| {
                    let mut x = (i as u64) << 32 | (r / 2) as u64;
                    let mut u = || {
                        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
                        let mut z = x;
                        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
                    };
                    let (a, b) = (u().max(1e-300), u());
                    ((-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()).exp()
                })
                .collect()
        })
        .collect()
}

/// Direction of order `[round][trader]`: opposite within each pair of rounds.
fn sized_dir(round: usize, trader: usize) -> bool {
    (round / 2 + trader + round).is_multiple_of(2)
}

/// Legs `(lane, amount_in, array starts)` of every `[round][trader]` order.
type SizedPlans = Vec<Vec<Vec<(usize, u64, Vec<i32>)>>>;

/// The router's legs `(lane, amount_in, array starts)` for every
/// `[round][trader]` order of `median * factor`, from one snapshot. The
/// router seed is shared within a pair of rounds, like the size.
fn sized_plans(
    books: &[amm_router::Book],
    cfg: &amm_router::MarketCfg,
    slot: u64,
    factors: &[Vec<f64>],
    median: f64,
    tax: u32,
) -> SizedPlans {
    let rc = amm_router::RouterConfig {
        split_tax_bps: tax,
        ..amm_router::RouterConfig::default()
    };
    factors
        .iter()
        .enumerate()
        .map(|(r, row)| {
            row.iter()
                .enumerate()
                .map(|(i, f)| {
                    let amount = (median * f).max(1.0) as u64;
                    let seed = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (r / 2) as u64;
                    amm_router::plan(books, cfg, slot, sized_dir(r, i), amount, rc, seed)
                        .expect("plan")
                        .legs
                        .into_iter()
                        .map(|l| (l.id as usize, l.amount_in, l.array_starts))
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// One order over all its legs in one v0 transaction through the market's
/// lookup table, the trader as fee payer, 60k CU per leg.
fn sized_tx(
    lanes: &[Shard],
    t: &Trader,
    a_to_b: bool,
    legs: &[(usize, u64, Vec<i32>)],
    alt: &solana_message::AddressLookupTableAccount,
    blockhash: &Hash,
) -> (String, String) {
    let (tin, tout) = if a_to_b {
        (t.token_a, t.token_b)
    } else {
        (t.token_b, t.token_a)
    };
    let mut cu = vec![2u8];
    cu.extend_from_slice(&(CU_LIMIT * legs.len() as u32).to_le_bytes());
    let mut ixs = vec![Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data: cu,
    }];
    for (lane, amount, starts) in legs {
        let s = &lanes[*lane];
        let accounts = sharded_amm::accounts::SwapExactIn {
            user: t.kp.pubkey(),
            market: s.market,
            registry: s.registry,
            shard: s.shard,
            vault_a: s.vault_a,
            vault_b: s.vault_b,
            user_token_in: tin,
            user_token_out: tout,
            vault_authority: s.vault_authority,
            token_program: TOKEN_PROGRAM,
        };
        // The leg is planned at its real size but executed at 1/1000 of
        // it: the CU of a swap that crosses no initialized tick does not
        // depend on the amount, so the cost charged to each shard is the
        // same, while prices stay inside the planned tick arrays (orders
        // planned from one snapshot land in another order).
        let args = SwapArgs {
            a_to_b,
            amount_in: (*amount / 1_000).max(1_000),
            min_amount_out: 0,
            sqrt_price_limit_x64: 0,
            max_rounding_dust: u64::MAX,
            deadline: i64::MAX,
        };
        let arrays: Vec<Pubkey> = starts
            .iter()
            .map(|st| common::tick_array_pda(&s.shard, *st))
            .collect();
        ixs.push(with_remaining(
            Instruction {
                program_id: sharded_amm::ID,
                accounts: accounts.to_account_metas(None),
                data: sharded_amm::instruction::SwapExactIn { args }.data(),
            },
            &arrays,
        ));
    }
    let msg = VersionedMessage::V0(
        solana_message::v0::Message::try_compile(
            &t.kp.pubkey(),
            &ixs,
            std::slice::from_ref(alt),
            *blockhash,
        )
        .unwrap(),
    );
    let body = msg.serialize();
    let sig = t.kp.sign_message(&body);
    let mut wire = vec![1u8];
    wire.extend_from_slice(sig.as_ref());
    wire.extend_from_slice(&body);
    (
        sig.to_string(),
        base64::engine::general_purpose::STANDARD.encode(wire),
    )
}

/// The router's view of the 8-lane market right now: every client of the
/// next burst reads this same snapshot, which is what makes herding likely.
fn books(rpc: &Rpc, lanes: &[Shard]) -> (Vec<amm_router::Book>, amm_router::MarketCfg) {
    use anchor_lang::AccountDeserialize;
    let market = rpc.fetch(&[lanes[0].market]).remove(0);
    let cfg = amm_router::MarketCfg::from_config(
        &sharded_amm::state::MarketConfig::try_deserialize(&mut market.as_slice()).unwrap(),
    );
    let books = lanes
        .iter()
        .enumerate()
        .map(|(id, l)| {
            let mut keys = vec![l.shard];
            for k in l.arrays_a_to_b.iter().chain(&l.arrays_b_to_a) {
                if !keys.contains(k) {
                    keys.push(*k);
                }
            }
            let data = rpc.fetch(&keys);
            let state =
                sharded_amm::state::ShardState::try_deserialize(&mut data[0].as_slice()).unwrap();
            let arrays =
                data[1..]
                    .iter()
                    .map(|d| unsafe {
                        std::ptr::read_unaligned(
                            d[8..].as_ptr() as *const sharded_amm::state::TickArray
                        )
                    })
                    .collect();
            amm_router::Book::from_state(id as u8, &state, arrays).unwrap()
        })
        .collect();
    (books, cfg)
}

/// Lane of every `[round][trader]` swap as `amm_router::plan` chooses it.
fn route(
    rpc: &Rpc,
    lanes: &[Shard],
    traders: &[Trader],
    rounds: usize,
    choices: usize,
) -> Vec<Vec<usize>> {
    let (books, cfg) = books(rpc, lanes);
    let slot = rpc.slot();
    let rc = amm_router::RouterConfig {
        choices,
        ..amm_router::RouterConfig::default()
    };
    (0..rounds)
        .map(|round| {
            traders
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let a_to_b = (i + round) % 2 == 0;
                    // Per-order seed, e.g. hash(user, blockhash) in the SDK.
                    let pk = t.kp.pubkey().to_bytes();
                    let seed = u64::from_le_bytes(pk[..8].try_into().unwrap())
                        ^ (round as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    let p = amm_router::plan(&books, &cfg, slot, a_to_b, 10_000, rc, seed).unwrap();
                    assert_eq!(p.legs.len(), 1, "small orders use one shard");
                    p.legs[0].id as usize
                })
                .collect()
        })
        .collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("bench dir"));
    let url = args.next().expect("rpc url");
    let rounds: usize = args.next().map(|v| v.parse().unwrap()).unwrap_or(8);
    let repeats: usize = args.next().map(|v| v.parse().unwrap()).unwrap_or(3);
    let out = args.next();

    let bench: Value =
        serde_json::from_slice(&std::fs::read(dir.join("bench.json")).unwrap()).unwrap();
    assert_eq!(bench["program_id"], sharded_amm::ID.to_string());
    let keys = |v: &Value| -> Vec<Pubkey> { v.as_array().unwrap().iter().map(key).collect() };
    // Lanes of the market built for each configuration (equal total capital).
    let markets: std::collections::BTreeMap<usize, Vec<Shard>> = bench["markets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            let shards = m["shards"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| Shard {
                    shard: key(&s["shard"]),
                    vault_a: key(&s["vault_a"]),
                    vault_b: key(&s["vault_b"]),
                    vault_authority: key(&s["vault_authority"]),
                    arrays_a_to_b: keys(&s["arrays_a_to_b"]),
                    arrays_b_to_a: keys(&s["arrays_b_to_a"]),
                    market: key(&m["market"]),
                    registry: key(&m["registry"]),
                })
                .collect();
            (m["n_shards"].as_u64().unwrap() as usize, shards)
        })
        .collect();
    let alts: std::collections::BTreeMap<usize, solana_message::AddressLookupTableAccount> = bench
        ["markets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["n_shards"].as_u64().unwrap() as usize,
                solana_message::AddressLookupTableAccount {
                    key: key(&m["alt"]),
                    addresses: keys(&m["alt_addresses"]),
                },
            )
        })
        .collect();
    let traders: Vec<Trader> = bench["traders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let bytes: Vec<u8> = t["secret"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| b.as_u64().unwrap() as u8)
                .collect();
            Trader {
                kp: Keypair::try_from(bytes.as_slice()).unwrap(),
                token_a: key(&t["token_a"]),
                token_b: key(&t["token_b"]),
            }
        })
        .collect();
    let rpc = Arc::new(Rpc::new(url));

    // Warm-up: one swap per shard, so program loading is not measured.
    let warm = run(
        &rpc,
        &markets[&8],
        &traders[..8],
        Load {
            n_shards: 8,
            rounds: 1,
            salt: 0,
            shared_payer: false,
            mode: Mode::RoundRobin,
            pick: None,
        },
    );
    assert_eq!(warm.ok, 8, "warm-up failed: {warm:?}");
    // Wait a few slots so the warm-up does not share blocks with run 1.
    let s0 = rpc.slot();
    while rpc.slot() < s0 + 3 {
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut results: Vec<Outcome> = Vec::new();
    // BENCH_ONLY_SIZED=1 runs only the sized orders (for iterating locally).
    let full_repeats = if std::env::var("BENCH_ONLY_SIZED").is_ok() {
        0
    } else {
        repeats
    };
    let salt = AtomicUsize::new(1);
    for rep in 0..full_repeats {
        // Alternate the order so drift over time does not favour one setting.
        let order: Vec<usize> = if rep % 2 == 0 {
            vec![1, 2, 4, 8]
        } else {
            vec![8, 4, 2, 1]
        };
        for n in order {
            let o = run(
                &rpc,
                &markets[&n],
                &traders,
                Load {
                    n_shards: n,
                    rounds,
                    salt: salt.fetch_add(1, Ordering::Relaxed) as u64,
                    shared_payer: false,
                    mode: Mode::RoundRobin,
                    pick: None,
                },
            );
            println!("{o:?}");
            results.push(o);
            let s0 = rpc.slot();
            while rpc.slot() < s0 + 3 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    // The real router instead of round-robin (parallelism audit P-01): every
    // client of a burst plans from the same fresh snapshot.
    for rep in 0..full_repeats {
        let modes = if rep % 2 == 0 {
            [Mode::Router, Mode::RouterBest]
        } else {
            [Mode::RouterBest, Mode::Router]
        };
        for mode in modes {
            let choices = if mode == Mode::Router { 2 } else { 0 };
            let pick = route(&rpc, &markets[&8], &traders, rounds, choices);
            let mut per_lane = [0usize; 8];
            pick.iter().flatten().for_each(|&l| per_lane[l] += 1);
            println!("{mode:?} lanes {per_lane:?}");
            let o = run(
                &rpc,
                &markets[&8],
                &traders,
                Load {
                    n_shards: 8,
                    rounds,
                    salt: salt.fetch_add(1, Ordering::Relaxed) as u64,
                    shared_payer: false,
                    mode,
                    pick: Some(Arc::new(pick)),
                },
            );
            println!("{o:?}");
            results.push(o);
            let s0 = rpc.slot();
            while rpc.slot() < s0 + 3 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    // Negative control: 8 shards, but one fee payer for every swap. The
    // payer is a writable account in every transaction, so all swaps contend
    // on it again, as they would on a shared vault.
    for _ in 0..full_repeats {
        let o = run(
            &rpc,
            &markets[&8],
            &traders,
            Load {
                n_shards: 8,
                rounds,
                salt: salt.fetch_add(1, Ordering::Relaxed) as u64,
                shared_payer: true,
                mode: Mode::RoundRobin,
                pick: None,
            },
        );
        println!("{o:?}");
        results.push(o);
        let s0 = rpc.slot();
        while rpc.slot() < s0 + 3 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // Orders of realistic size (fee simulation, docs/sim/fee-schedule-
    // analysis.md): sizes are scaled until the router with a 10 bps tax
    // uses as many shards per order as in the simulation of SOL/USDC 0.05%
    // with $1M (1.73), then the same orders run with the 3 bps default and
    // on one shard holding all the capital. Every order is one transaction
    // over all its legs, so its full cost is charged to every shard it
    // writes (audit P-02).
    const TARGET_LEGS: f64 = 1.73;
    let (bs8, cfg8) = books(&rpc, &markets[&8]);
    let slot8 = rpc.slot();
    let factors = size_factors(rounds, traders.len());
    let mean_legs = |plans: &SizedPlans| {
        let n: usize = plans.iter().map(Vec::len).sum();
        plans.iter().flatten().map(Vec::len).sum::<usize>() as f64 / n as f64
    };
    let sample = &factors[..2.min(factors.len())];
    let (mut lo, mut hi) = (1e6f64.ln(), 1e14f64.ln());
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if mean_legs(&sized_plans(&bs8, &cfg8, slot8, sample, mid.exp(), 10)) < TARGET_LEGS {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let median_size = lo.exp();
    println!("sized orders: median {median_size:.0} units");
    // The lookup tables were written at slot 0; right after start-up they
    // do not resolve yet (seen on a fresh validator), so wait a few dozen
    // slots, which the full benchmark has long passed by now.
    while rpc.slot() < 64 {
        std::thread::sleep(Duration::from_millis(100));
    }
    let (bs1, cfg1) = books(&rpc, &markets[&1]);
    for rep in 0..repeats {
        let mut order = vec![(1usize, 0u32), (8, 10), (8, 3)];
        if rep % 2 == 1 {
            order.reverse();
        }
        for (n, tax) in order {
            let (bs, cfg) = if n == 1 { (&bs1, &cfg1) } else { (&bs8, &cfg8) };
            // Plans from the snapshot before any sized burst; each pair of
            // rounds leaves prices where they were, up to rounding.
            let plans = sized_plans(bs, cfg, slot8, &factors, median_size, tax);
            let legs = mean_legs(&plans);
            let blockhash = rpc.blockhash();
            let mut txs = Vec::new();
            for (r, row) in plans.iter().enumerate() {
                for (i, l) in row.iter().enumerate() {
                    txs.push(sized_tx(
                        &markets[&n],
                        &traders[i],
                        sized_dir(r, i),
                        l,
                        &alts[&n],
                        &blockhash,
                    ));
                }
            }
            let mut o = measure(&rpc, txs, n, Mode::Sized { tax }, false);
            o.legs = legs;
            println!("{o:?}");
            results.push(o);
            let s0 = rpc.slot();
            while rpc.slot() < s0 + 3 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let rr = Mode::RoundRobin;
    let configs: Vec<(usize, bool, Mode, &str)> = vec![
        (1, false, rr, "1 shard"),
        (2, false, rr, "2 shard-e"),
        (4, false, rr, "4 shard-e"),
        (8, false, rr, "8 shard-e"),
        (
            8,
            false,
            Mode::Router,
            "8 shard-e, router (power of two choices)",
        ),
        (
            8,
            false,
            Mode::RouterBest,
            "8 shard-e, router determinist (para ndreqjes)",
        ),
        (
            8,
            true,
            rr,
            "8 shard-e, payer i përbashkët (kontroll negativ)",
        ),
    ];
    let median = |mut v: Vec<usize>| -> usize {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let base = median(
        results
            .iter()
            .filter(|o| o.shards == 1 && !o.shared_payer && o.mode == rr)
            .map(|o| o.max_per_slot)
            .collect(),
    );
    let mut md = String::from(
        "| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |\n|---|---:|---:|\n",
    );
    for &(n, shared, mode, label) in &configs {
        let m = median(
            results
                .iter()
                .filter(|o| o.shards == n && o.shared_payer == shared && o.mode == mode)
                .map(|o| o.max_per_slot)
                .collect(),
        );
        md += &format!("| {label} | {m} | {:.2} |\n", m as f64 / base.max(1) as f64);
    }
    md += "\n| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for &(n, shared, mode, label) in &configs {
        for (i, o) in results
            .iter()
            .filter(|o| o.shards == n && o.shared_payer == shared && o.mode == mode)
            .enumerate()
        {
            md += &format!(
                "| {label} | {} | {} | {} | {} | {} | {} | {:.1} | {} / {} | {} |\n",
                i + 1,
                o.sent,
                o.ok,
                o.failed,
                o.missing,
                o.max_per_slot,
                o.busiest_cu as f64 / 1e6,
                o.lat_p50,
                o.lat_p95,
                o.send_ms
            );
        }
    }
    let sized: Vec<(usize, u32, &str)> = vec![
        (1, 0, "1 shard (gjithë kapitali)"),
        (8, 10, "8 shard-e, router taksë 10 bps"),
        (8, 3, "8 shard-e, router taksë 3 bps"),
    ];
    let pick = |n: usize, tax: u32| {
        results
            .iter()
            .filter(move |o| o.shards == n && o.mode == Mode::Sized { tax })
    };
    let sized_base = median(pick(1, 0).map(|o| o.max_per_slot).collect());
    md += &format!(
        "\nUrdhra me madhësi reale (mediana {median_size:.0} njësi; router-i me 10 bps përdor {TARGET_LEGS} shard-e për urdhër, si në simulimin e tarifave), çdo urdhër 1 transaksion v0 me ALT. Çdo hap ekzekutohet me 1/1000 e shumës së planifikuar: CU-të janë të njëjta (asnjë tick i inicializuar nuk kalohet), çmimet mbeten brenda tick arrays të planifikuara:\n\n| konfigurimi | shard-e për urdhër | urdhra/slot maks. (mediana) | herë sa 1 shard | të suksesshme / dërguara | CU në slot-in maks. (M) |\n|---|---:|---:|---:|---:|---:|\n"
    );
    for &(n, tax, label) in &sized {
        let m = median(pick(n, tax).map(|o| o.max_per_slot).collect());
        let legs = pick(n, tax).map(|o| o.legs).sum::<f64>() / repeats as f64;
        let ok: usize = pick(n, tax).map(|o| o.ok).sum();
        let sent: usize = pick(n, tax).map(|o| o.sent).sum();
        let cu = pick(n, tax).map(|o| o.busiest_cu).max().unwrap_or(0);
        md += &format!(
            "| {label} | {legs:.2} | {m} | {:.2} | {ok} / {sent} | {:.1} |\n",
            m as f64 / sized_base.max(1) as f64,
            cu as f64 / 1e6
        );
    }
    print!("{md}");
    if let Some(path) = out {
        std::fs::write(path, md).unwrap();
    }
    // A failed swap means the benchmark itself is wrong (bad accounts or
    // state), not that the network was slow.
    assert!(
        results.iter().all(|o| o.failed == 0),
        "some swaps failed: {:?}",
        results.iter().find(|o| o.failed > 0)
    );
    // Sized orders are planned from one snapshot and land in another order,
    // so a few may leave their tick arrays; more than 2% means a bad setup.
    for o in results
        .iter()
        .filter(|o| matches!(o.mode, Mode::Sized { .. }))
    {
        assert!(o.failed * 50 <= o.sent, "sized orders failed: {o:?}");
    }
}
