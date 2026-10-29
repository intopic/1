//! Rebalancer bot (ADR-0006), off chain, run against a local
//! `solana-test-validator` with the 8-shard market from `bench_setup`.
//!
//! Each round the bot:
//! 1. reads the market, the 8 shards and the tick arrays around each price
//!    in one `getMultipleAccounts` (one slot, one consistent snapshot);
//! 2. plans up to one `rebalance_pair` per listed rebalancer key on disjoint
//!    shard pairs (`amm_router::rebalance_round`), each paying at least a
//!    floor per profit token (`--min-a`, `--min-b`): what one transaction
//!    costs the bot in that token, so it does not send one that loses money;
//! 3. sends them at once, each signed and paid by its own key, with
//!    `min_profit` onchain at no less than the floor (a stale snapshot makes
//!    the transaction fail instead of paying less than it costs);
//! 4. waits for every signature and handles the result with
//!    `amm_router::recovery::after_send`: a failure is decoded and
//!    classified, an unseen transaction is resent with the same bytes while
//!    its blockhash is valid and given up only once it expired and the full
//!    history shows it never ran.
//!
//! The run has two phases. Calm: eight traders push the shards apart (half
//! up, half down), then only the bot acts, so every rebalance must land and
//! the gap must close. Noisy: traders keep swapping on random shards while
//! the bot runs, so some snapshots go stale; every failure must decode to a
//! known program error, and the bot's token balances must never go down.
//!
//! The floors here are in test-token units. On a real market set them from
//! the fee the bot pays (5 000 lamports per signature plus any priority fee)
//! times the token's price in lamports, plus a margin.
//!
//! This measures one machine and a single-node validator: no gossip, no
//! leader rotation, no competing bots, no priority-fee market.
//!
//! Usage: cargo run --release --example rebalancer -- <bench-dir> <rpc-url> \
//!        [out.md] [--min-a N] [--min-b N] [--noise N]

#[path = "../tests/common/mod.rs"]
mod common;
#[path = "shared/rpc.rs"]
mod rpc;

use amm_router::recovery::{after_send, AfterSend, TxStatus};
use amm_router::{rebalance_round, Book, MarketCfg, RebalancePlan};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use base64::Engine;
use common::{tick_array_pda, with_remaining, Pubkey, TOKEN_PROGRAM};
use rpc::Rpc;
use serde_json::{json, Value};
use sharded_amm::instructions::{RebalanceArgs, SwapArgs};
use sharded_amm::state::{array_span, array_start_for, MarketConfig, ShardState, TickArray};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_message::{Hash, Message, VersionedMessage};
use solana_signer::Signer;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const COMPUTE_BUDGET: Pubkey = anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111");
/// A rebalance is two swaps in one instruction; see the measured CU in the
/// report.
const CU_LIMIT: u32 = 300_000;
/// Arrays read around each shard's price, on each side of the current one.
const ARRAYS_AROUND: i32 = 3;
/// A rebalance pays only while the gap is wider than the two shards' fees,
/// so the gap left is that band plus what the floors leave: this margin.
const BAND_MARGIN_BPS: f64 = 5.0;

fn key(v: &Value) -> Pubkey {
    Pubkey::from_str(v.as_str().unwrap()).unwrap()
}

fn keypair(v: &Value) -> Keypair {
    let bytes: Vec<u8> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_u64().unwrap() as u8)
        .collect();
    Keypair::try_from(bytes.as_slice()).unwrap()
}

struct Wallet {
    kp: Keypair,
    token_a: Pubkey,
    token_b: Pubkey,
}

impl Wallet {
    fn from_json(v: &Value) -> Self {
        Self {
            kp: keypair(&v["secret"]),
            token_a: key(&v["token_a"]),
            token_b: key(&v["token_b"]),
        }
    }
}

struct Lane {
    shard: Pubkey,
    vault_a: Pubkey,
    vault_b: Pubkey,
    vault_authority: Pubkey,
}

struct Market {
    market: Pubkey,
    registry: Pubkey,
    lanes: Vec<Lane>,
}

/// Signs `ixs` with `payer` (fee payer and only signer) behind a CU limit.
fn encode(ixs: Vec<Instruction>, payer: &Keypair, blockhash: &Hash) -> (String, String) {
    let mut cu = vec![2u8];
    cu.extend_from_slice(&CU_LIMIT.to_le_bytes());
    let mut all = vec![Instruction {
        program_id: COMPUTE_BUDGET,
        accounts: vec![],
        data: cu,
    }];
    all.extend(ixs);
    let msg = VersionedMessage::Legacy(Message::new_with_blockhash(
        &all,
        Some(&payer.pubkey()),
        blockhash,
    ));
    let body = msg.serialize();
    let sig = payer.sign_message(&body);
    let mut wire = vec![1u8];
    wire.extend_from_slice(sig.as_ref());
    wire.extend_from_slice(&body);
    (
        sig.to_string(),
        base64::engine::general_purpose::STANDARD.encode(wire),
    )
}

fn send(rpc: &Rpc, wire: &str) {
    // A rejected send (e.g. a duplicate) is fine: the status decides.
    let _ = rpc.try_call(
        "sendTransaction",
        json!([wire, {"encoding": "base64", "skipPreflight": true, "maxRetries": 0}]),
    );
}

fn swap_ix(
    m: &Market,
    lane: usize,
    t: &Wallet,
    a_to_b: bool,
    amount: u64,
    tick: i32,
    spacing: u16,
) -> Instruction {
    let l = &m.lanes[lane];
    let (tin, tout) = if a_to_b {
        (t.token_a, t.token_b)
    } else {
        (t.token_b, t.token_a)
    };
    let accounts = sharded_amm::accounts::SwapExactIn {
        user: t.kp.pubkey(),
        market: m.market,
        registry: m.registry,
        shard: l.shard,
        vault_a: l.vault_a,
        vault_b: l.vault_b,
        user_token_in: tin,
        user_token_out: tout,
        vault_authority: l.vault_authority,
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
    let span = array_span(spacing);
    let start = array_start_for(tick, spacing);
    let step = if a_to_b { -span } else { span };
    let arrays: Vec<Pubkey> = (0..2)
        .map(|k| tick_array_pda(&l.shard, start + k * step))
        .collect();
    with_remaining(
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::SwapExactIn { args }.data(),
        },
        &arrays,
    )
}

fn rebalance_ix(m: &Market, r: &RebalancePlan, caller: &Wallet, min_profit: u64) -> Instruction {
    let (x, y) = (&m.lanes[r.lo as usize], &m.lanes[r.hi as usize]);
    let arrays_x: Vec<Pubkey> = r
        .lo_array_starts
        .iter()
        .map(|s| tick_array_pda(&x.shard, *s))
        .collect();
    let arrays_y: Vec<Pubkey> = r
        .hi_array_starts
        .iter()
        .map(|s| tick_array_pda(&y.shard, *s))
        .collect();
    let accounts = sharded_amm::accounts::RebalancePair {
        caller: caller.kp.pubkey(),
        market: m.market,
        registry: m.registry,
        shard_x: x.shard,
        vault_x_a: x.vault_a,
        vault_x_b: x.vault_b,
        vault_authority_x: x.vault_authority,
        shard_y: y.shard,
        vault_y_a: y.vault_a,
        vault_y_b: y.vault_b,
        vault_authority_y: y.vault_authority,
        caller_token: if r.input_is_a {
            caller.token_a
        } else {
            caller.token_b
        },
        token_program: TOKEN_PROGRAM,
    };
    let args = RebalanceArgs {
        input_is_a: r.input_is_a,
        amount_in: r.amount_in,
        min_profit,
        arrays_x: arrays_x.len() as u8,
        deadline: i64::MAX,
    };
    with_remaining(
        Instruction {
            program_id: sharded_amm::ID,
            accounts: accounts.to_account_metas(None),
            data: sharded_amm::instruction::RebalancePair { args }.data(),
        },
        &[arrays_x, arrays_y].concat(),
    )
}

/// One consistent snapshot of the market: books, config and the slot it
/// was read at.
fn snapshot(rpc: &Rpc, m: &Market) -> (Vec<Book>, MarketCfg, u64) {
    let mut keys = vec![m.market];
    keys.extend(m.lanes.iter().map(|l| l.shard));
    let (_, data) = rpc.fetch_at(&keys);
    let cfg_acc = MarketConfig::try_deserialize(&mut data[0].as_ref().unwrap().as_slice()).unwrap();
    let cfg = MarketCfg::from_config(&cfg_acc);
    let spacing = cfg.tick_spacing;
    let span = array_span(spacing);
    // Arrays around each price come from the shard read just now; the
    // second read below also re-reads the shards, and that read is the one
    // used, so shards and arrays are from the same slot.
    let ticks: Vec<i32> = data[1..]
        .iter()
        .map(|d| {
            ShardState::try_deserialize(&mut d.as_ref().unwrap().as_slice())
                .unwrap()
                .tick_current
        })
        .collect();
    let mut keys = keys.clone();
    let mut owner = Vec::new();
    for (i, l) in m.lanes.iter().enumerate() {
        let start = array_start_for(ticks[i], spacing);
        for k in -ARRAYS_AROUND..=ARRAYS_AROUND {
            keys.push(tick_array_pda(&l.shard, start + k * span));
            owner.push(i);
        }
    }
    let (slot, data) = rpc.fetch_at(&keys);
    let mut arrays: Vec<Vec<TickArray>> = vec![Vec::new(); m.lanes.len()];
    for (d, &i) in data[1 + m.lanes.len()..].iter().zip(&owner) {
        if let Some(d) = d {
            arrays[i]
                .push(unsafe { std::ptr::read_unaligned(d[8..].as_ptr() as *const TickArray) });
        }
    }
    let books = data[1..=m.lanes.len()]
        .iter()
        .zip(arrays)
        .enumerate()
        .map(|(i, (d, a))| {
            let s = ShardState::try_deserialize(&mut d.as_ref().unwrap().as_slice()).unwrap();
            Book::from_state(i as u8, &s, a).unwrap()
        })
        .collect();
    (books, cfg, slot)
}

/// Widest price gap between two shards, in bps of price.
fn gap_bps(books: &[Book]) -> f64 {
    let q = |b: &Book| b.pool.sqrt_price as f64;
    let hi = books.iter().map(q).fold(f64::MIN, f64::max);
    let lo = books.iter().map(q).fold(f64::MAX, f64::min);
    ((hi / lo).powi(2) - 1.0) * 1e4
}

fn token_amount(d: &[u8]) -> u64 {
    u64::from_le_bytes(d[64..72].try_into().unwrap())
}

/// Token balances (A, B) of every rebalancer.
fn balances(rpc: &Rpc, rs: &[Wallet]) -> Vec<(u64, u64)> {
    let keys: Vec<Pubkey> = rs.iter().flat_map(|r| [r.token_a, r.token_b]).collect();
    rpc.fetch(&keys)
        .chunks(2)
        .map(|c| (token_amount(&c[0]), token_amount(&c[1])))
        .collect()
}

/// The status of each signature, from the full history.
fn statuses(rpc: &Rpc, sigs: &[String]) -> Vec<Option<(u64, TxStatus)>> {
    let r = rpc.call(
        "getSignatureStatuses",
        json!([sigs, {"searchTransactionHistory": true}]),
    );
    r["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            if v.is_null() {
                return None;
            }
            let done =
                v["confirmationStatus"] == "confirmed" || v["confirmationStatus"] == "finalized";
            if !done {
                return None;
            }
            let slot = v["slot"].as_u64().unwrap();
            let err = &v["err"];
            let st = if err.is_null() {
                TxStatus::Landed
            } else if let Some(code) = err["InstructionError"][1]["Custom"].as_u64() {
                TxStatus::Failed(code as u32)
            } else {
                TxStatus::FailedOther
            };
            Some((slot, st))
        })
        .collect()
}

#[derive(Debug, Default, Clone)]
struct Round {
    phase: &'static str,
    gap_before: f64,
    gap_after: f64,
    sent: usize,
    landed: usize,
    /// Program error name of each failure, with its recovery class.
    failures: Vec<String>,
    resent: usize,
    expired: usize,
    slots: BTreeSet<u64>,
    max_in_one_slot: usize,
    profit_a: u64,
    profit_b: u64,
    max_cu: u64,
    fees_lamports: u64,
}

/// Sends `plans` (plan `k` by rebalancer `k`), waits for every one and
/// returns what happened.
fn execute(
    rpc: &Rpc,
    m: &Market,
    rs: &[Wallet],
    plans: &[RebalancePlan],
    floors: [u64; 2],
) -> Round {
    let (blockhash, last_valid) = rpc.blockhash_valid_until();
    let txs: Vec<(String, String)> = plans
        .iter()
        .zip(rs)
        .map(|(r, w)| {
            let floor = floors[usize::from(!r.input_is_a)];
            // Accept a little drift since the snapshot, never less than
            // what the transaction costs.
            let min_profit = (r.profit - r.profit / 10).max(floor);
            encode(vec![rebalance_ix(m, r, w, min_profit)], &w.kp, &blockhash)
        })
        .collect();
    std::thread::scope(|s| {
        for (_, wire) in &txs {
            s.spawn(move || send(rpc, wire));
        }
    });
    let sigs: Vec<String> = txs.iter().map(|t| t.0.clone()).collect();
    let mut out = Round {
        sent: sigs.len(),
        ..Round::default()
    };
    let mut done: Vec<Option<(u64, TxStatus)>> = vec![None; sigs.len()];
    let mut gave_up = vec![false; sigs.len()];
    let mut last_resend = Instant::now();
    loop {
        for (i, s) in statuses(rpc, &sigs).into_iter().enumerate() {
            if done[i].is_none() {
                done[i] = s;
            }
        }
        let height = rpc.block_height();
        let mut pending = false;
        for i in 0..sigs.len() {
            if done[i].is_some() || gave_up[i] {
                continue;
            }
            let arrays = plans[i].lo_array_starts.len() + plans[i].hi_array_starts.len();
            match after_send(
                TxStatus::NotFound { history: true },
                height,
                last_valid,
                arrays,
            ) {
                AfterSend::ResendSame => {
                    pending = true;
                    if last_resend.elapsed() > Duration::from_secs(2) {
                        send(rpc, &txs[i].1);
                        out.resent += 1;
                    }
                }
                AfterSend::Expired => {
                    gave_up[i] = true;
                    out.expired += 1;
                }
                other => unreachable!("{other:?}"),
            }
        }
        if last_resend.elapsed() > Duration::from_secs(2) {
            last_resend = Instant::now();
        }
        if !pending {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let mut per_slot: BTreeMap<u64, usize> = BTreeMap::new();
    for (i, d) in done.iter().enumerate() {
        let Some((slot, st)) = d else { continue };
        out.slots.insert(*slot);
        match after_send(*st, 0, u64::MAX, 0) {
            AfterSend::Done => {
                out.landed += 1;
                *per_slot.entry(*slot).or_default() += 1;
            }
            AfterSend::Recover(class) => {
                let name = match st {
                    TxStatus::Failed(code) => amm_router::recovery::decode(*code)
                        .map_or(format!("custom {code}"), |e| format!("{e:?}")),
                    _ => "not a program error".to_string(),
                };
                out.failures.push(format!("{name} → {class:?}"));
            }
            other => unreachable!("{other:?}"),
        }
        // CU and fee as the validator charged them.
        let t = rpc.call(
            "getTransaction",
            json!([sigs[i], {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": 0}]),
        );
        out.max_cu = out
            .max_cu
            .max(t["meta"]["computeUnitsConsumed"].as_u64().unwrap_or(0));
        out.fees_lamports += t["meta"]["fee"].as_u64().unwrap_or(0);
    }
    out.max_in_one_slot = per_slot.values().copied().max().unwrap_or(0);
    out
}

/// Runs bot rounds until no plan clears the floors (or `stop` says the
/// noise is over and a round finds nothing).
fn bot(
    rpc: &Rpc,
    m: &Market,
    rs: &[Wallet],
    floors: [u64; 2],
    phase: &'static str,
    noise_running: &AtomicBool,
    max_rounds: usize,
) -> Vec<Round> {
    let mut rounds = Vec::new();
    for _ in 0..max_rounds {
        let before = balances(rpc, rs);
        let (books, cfg, slot) = snapshot(rpc, m);
        let plans = rebalance_round(&books, &cfg, slot, floors, rs.len());
        if plans.is_empty() {
            if noise_running.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(400));
                continue;
            }
            break;
        }
        let mut r = execute(rpc, m, rs, &plans, floors);
        r.phase = phase;
        r.gap_before = gap_bps(&books);
        r.gap_after = gap_bps(&snapshot(rpc, m).0);
        let after = balances(rpc, rs);
        for (b, a) in before.iter().zip(&after) {
            assert!(
                a.0 >= b.0 && a.1 >= b.1,
                "a rebalancer lost tokens: {b:?} -> {a:?}"
            );
            r.profit_a += a.0 - b.0;
            r.profit_b += a.1 - b.1;
        }
        println!("{r:?}");
        rounds.push(r);
    }
    rounds
}

fn wait_slots(rpc: &Rpc, n: u64) {
    let s0 = rpc.slot();
    while rpc.slot() < s0 + n {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Sends `swaps` and waits until each has a status.
fn send_and_wait(rpc: &Rpc, swaps: Vec<(String, String)>) {
    std::thread::scope(|s| {
        for (_, w) in &swaps {
            s.spawn(move || send(rpc, w));
        }
    });
    let sigs: Vec<String> = swaps.into_iter().map(|t| t.0).collect();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let st = statuses(rpc, &sigs);
        if st.iter().all(Option::is_some) {
            assert!(
                st.iter().all(|s| matches!(s, Some((_, TxStatus::Landed)))),
                "a trader swap failed: {st:?}"
            );
            return;
        }
        assert!(Instant::now() < deadline, "trader swaps did not land");
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("bench dir"));
    let url = args.next().expect("rpc url");
    let mut out = None;
    let (mut min_a, mut min_b, mut noise) = (5_000u64, 5_000u64, 120usize);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--min-a" => min_a = args.next().unwrap().parse().unwrap(),
            "--min-b" => min_b = args.next().unwrap().parse().unwrap(),
            "--noise" => noise = args.next().unwrap().parse().unwrap(),
            _ => out = Some(a),
        }
    }
    let floors = [min_a, min_b];

    let bench: Value =
        serde_json::from_slice(&std::fs::read(dir.join("bench.json")).unwrap()).unwrap();
    assert_eq!(bench["program_id"], sharded_amm::ID.to_string());
    let mj = bench["markets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["n_shards"] == 8)
        .expect("8-shard market");
    let m = Market {
        market: key(&mj["market"]),
        registry: key(&mj["registry"]),
        lanes: mj["shards"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| Lane {
                shard: key(&s["shard"]),
                vault_a: key(&s["vault_a"]),
                vault_b: key(&s["vault_b"]),
                vault_authority: key(&s["vault_authority"]),
            })
            .collect(),
    };
    let rs: Vec<Wallet> = mj["rebalancers"]
        .as_array()
        .expect("bench_setup lists rebalancers")
        .iter()
        .map(Wallet::from_json)
        .collect();
    assert_eq!(rs.len(), 4);
    let traders: Vec<Wallet> = bench["traders"]
        .as_array()
        .unwrap()
        .iter()
        .map(Wallet::from_json)
        .collect();
    let rpc = Arc::new(Rpc::new(url));
    let (books0, cfg, _) = snapshot(&rpc, &m);
    let spacing = cfg.tick_spacing;
    let band = 2.0 * f64::from(cfg.base_fee_ppm) / 100.0;
    let bound = band + BAND_MARGIN_BPS;
    assert!(gap_bps(&books0) < 1.0, "shards start at one price");

    // Calm phase: trader i pushes shard i, even ones down, odd ones up, by
    // different amounts, so there are four or more gaps to close.
    let bh = rpc.blockhash();
    let push: Vec<(String, String)> = (0..8usize)
        .map(|i| {
            let a_to_b = i % 2 == 0;
            let amount = 300_000_000_000 + i as u64 * 40_000_000_000;
            let tick = books0[i].pool.tick;
            encode(
                vec![swap_ix(&m, i, &traders[i], a_to_b, amount, tick, spacing)],
                &traders[i].kp,
                &bh,
            )
        })
        .collect();
    send_and_wait(&rpc, push);
    wait_slots(&rpc, 2);
    let idle = AtomicBool::new(false);
    let calm = bot(&rpc, &m, &rs, floors, "calm", &idle, 20);
    let (books1, _, _) = snapshot(&rpc, &m);
    let calm_gap = gap_bps(&books1);

    // Noisy phase: traders swap on random shards while the bot runs.
    let running = Arc::new(AtomicBool::new(true));
    let sent_noise = Arc::new(AtomicUsize::new(0));
    let noisy = std::thread::scope(|s| {
        let (rpc2, m2, tr, run2, sn) = (&rpc, &m, &traders, running.clone(), sent_noise.clone());
        s.spawn(move || {
            let mut x = 0x2545_f491_4f6c_dd1du64;
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let mut bh = rpc2.blockhash();
            for k in 0..noise {
                if k % 32 == 0 {
                    bh = rpc2.blockhash();
                }
                let lane = (next() % 8) as usize;
                let t = &tr[8 + (next() % (tr.len() as u64 - 8)) as usize];
                let a_to_b = next() % 2 == 0;
                let amount = 10_000_000_000 + next() % 90_000_000_000;
                // Arrays from the last calm read; a swap that leaves them
                // fails, which the noise accepts.
                let tick = books1[lane].pool.tick;
                let (_, w) = encode(
                    vec![swap_ix(m2, lane, t, a_to_b, amount, tick, spacing)],
                    &t.kp,
                    &bh,
                );
                send(rpc2, &w);
                sn.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(50));
            }
            run2.store(false, Ordering::Relaxed);
        });
        bot(&rpc, &m, &rs, floors, "noisy", &running, 200)
    });
    // After the noise, the bot converges again.
    wait_slots(&rpc, 2);
    let settle = bot(&rpc, &m, &rs, floors, "settle", &idle, 20);
    let (books2, _, _) = snapshot(&rpc, &m);
    let final_gap = gap_bps(&books2);

    // Report.
    let all: Vec<&Round> = calm.iter().chain(&noisy).chain(&settle).collect();
    let mut md = String::new();
    md += &format!(
        "Floors: {min_a} A, {min_b} B per transaction. Four rebalancer keys, each its own fee payer. Noise: {} swaps.\n\n",
        sent_noise.load(Ordering::Relaxed)
    );
    md += "| faza | raundi | hendeku para (bps) | pas (bps) | dërguar | zbritën | dështuan | ridërguar | skaduan | slot-e | maks. në 1 slot | fitimi A | fitimi B | CU maks./tx | tarifa (lamports) |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for (i, r) in all.iter().enumerate() {
        md += &format!(
            "| {} | {} | {:.1} | {:.1} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.phase,
            i + 1,
            r.gap_before,
            r.gap_after,
            r.sent,
            r.landed,
            r.failures.len(),
            r.resent,
            r.expired,
            r.slots.len(),
            r.max_in_one_slot,
            r.profit_a,
            r.profit_b,
            r.max_cu,
            r.fees_lamports
        );
    }
    let fails: BTreeMap<&String, usize> =
        all.iter()
            .flat_map(|r| &r.failures)
            .fold(BTreeMap::new(), |mut m, f| {
                *m.entry(f).or_default() += 1;
                m
            });
    md += "\n| dështimi → klasa | herë |\n|---|---:|\n";
    for (f, n) in &fails {
        md += &format!("| {f} | {n} |\n");
    }
    let gap0 = calm.first().map_or(0.0, |r| r.gap_before);
    md += &format!(
        "\nHendeku: {gap0:.1} bps pas shtyrjes, {calm_gap:.1} bps pas fazës së qetë, {final_gap:.1} bps në fund. Brezi i tarifave (dy tarifa) është {band:.1} bps: brenda tij asnjë rebalancim nuk paguan.\n"
    );
    print!("{md}");
    if let Some(path) = out {
        std::fs::write(path, &md).unwrap();
    }

    // Calm: every rebalance landed, the gap closed, rounds used parallel pairs.
    assert!(!calm.is_empty(), "the bot found nothing to rebalance");
    assert!(
        calm.iter()
            .all(|r| r.failures.is_empty() && r.expired == 0 && r.landed == r.sent),
        "calm phase failures: {calm:?}"
    );
    assert!(calm.iter().any(|r| r.sent >= 2), "no round used two keys");
    assert!(
        calm_gap < bound,
        "calm gap {calm_gap:.1} bps, bound {bound:.1}"
    );
    assert!(calm_gap * 2.0 < gap0, "{calm_gap:.1} vs {gap0:.1}");
    // Noisy: a failure only ever means the snapshot went stale (refresh and
    // plan again), and the bot converges once the noise stops.
    assert!(fails.keys().all(|f| f.ends_with("Refresh")), "{fails:?}");
    assert!(
        final_gap < bound,
        "final gap {final_gap:.1} bps, bound {bound:.1}"
    );
}
