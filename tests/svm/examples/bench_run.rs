//! Validator benchmark, step 2 (acceptance A-10): sends the same number of
//! independent swaps to a local `solana-test-validator` spread over 1, 2, 4
//! and 8 shards, and measures how many land per slot.
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

use anchor_lang::{InstructionData, ToAccountMetas};
use base64::Engine;
use common::{with_remaining, Pubkey, TOKEN_PROGRAM};
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

struct Rpc {
    url: String,
    agent: ureq::Agent,
}

impl Rpc {
    /// Retries transport errors and "block not available yet" (-32004).
    fn call(&self, method: &str, params: Value) -> Value {
        const ATTEMPTS: usize = 40;
        for attempt in 1..=ATTEMPTS {
            let r = self.agent.post(&self.url).send_json(json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }));
            let retry = match r {
                Ok(resp) => {
                    let v: Value = resp.into_json().unwrap();
                    match v.get("error") {
                        None => return v["result"].clone(),
                        Some(e) if e["code"] == -32004 => format!("{e}"),
                        Some(e) => panic!("{method}: {e}"),
                    }
                }
                Err(e) => format!("{e}"),
            };
            if attempt == ATTEMPTS {
                panic!("{method}: {retry}");
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        unreachable!()
    }

    fn blockhash(&self) -> Hash {
        let r = self.call("getLatestBlockhash", json!([{"commitment": "confirmed"}]));
        Hash::from_str(r["value"]["blockhash"].as_str().unwrap()).unwrap()
    }

    fn slot(&self) -> u64 {
        self.call("getSlot", json!([{"commitment": "processed"}]))
            .as_u64()
            .unwrap()
    }
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
}

/// One swap by `t`; `payer` pays the fee (the trader itself, or a shared
/// payer for the negative control).
fn swap_tx(
    bench: &Value,
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
        market: key(&bench["market"]),
        registry: key(&bench["registry"]),
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

#[derive(Clone, Debug)]
struct Outcome {
    shards: usize,
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
}

/// One measured burst: `rounds` swaps per trader over the first `n_shards`.
struct Load {
    n_shards: usize,
    rounds: usize,
    /// Makes every amount, and so every signature, unique across bursts.
    salt: u64,
    /// Negative control: one fee payer for every swap.
    shared_payer: bool,
}

fn run(rpc: &Arc<Rpc>, bench: &Value, shards: &[Shard], traders: &[Trader], load: Load) -> Outcome {
    let Load {
        n_shards,
        rounds,
        salt,
        shared_payer,
    } = load;
    let blockhash = rpc.blockhash();
    let mut txs = Vec::new();
    for round in 0..rounds {
        for (i, t) in traders.iter().enumerate() {
            let s = &shards[i % n_shards];
            let a_to_b = (i + round) % 2 == 0;
            // Distinct amounts keep every signature unique across runs.
            let amount = 10_000 + salt * 1_000 + round as u64;
            let payer = if shared_payer { &traders[0].kp } else { &t.kp };
            txs.push(swap_tx(bench, s, t, payer, a_to_b, amount, &blockhash));
        }
    }
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
        sent: sigs.len(),
        ok: ok_slots.len(),
        failed,
        missing,
        max_per_slot: per_slot.values().copied().max().unwrap_or(0),
        send_ms,
    }
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
    let shards: Vec<Shard> = bench["shards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Shard {
            shard: key(&s["shard"]),
            vault_a: key(&s["vault_a"]),
            vault_b: key(&s["vault_b"]),
            vault_authority: key(&s["vault_authority"]),
            arrays_a_to_b: s["arrays_a_to_b"]
                .as_array()
                .unwrap()
                .iter()
                .map(key)
                .collect(),
            arrays_b_to_a: s["arrays_b_to_a"]
                .as_array()
                .unwrap()
                .iter()
                .map(key)
                .collect(),
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
    let rpc = Arc::new(Rpc {
        url,
        agent: ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build(),
    });

    // Warm-up: one swap per shard, so program loading is not measured.
    let warm = run(
        &rpc,
        &bench,
        &shards,
        &traders[..8],
        Load {
            n_shards: 8,
            rounds: 1,
            salt: 0,
            shared_payer: false,
        },
    );
    assert_eq!(warm.ok, 8, "warm-up failed: {warm:?}");
    // Wait a few slots so the warm-up does not share blocks with run 1.
    let s0 = rpc.slot();
    while rpc.slot() < s0 + 3 {
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut results: Vec<Outcome> = Vec::new();
    let salt = AtomicUsize::new(1);
    for rep in 0..repeats {
        // Alternate the order so drift over time does not favour one setting.
        let order: Vec<usize> = if rep % 2 == 0 {
            vec![1, 2, 4, 8]
        } else {
            vec![8, 4, 2, 1]
        };
        for n in order {
            let o = run(
                &rpc,
                &bench,
                &shards,
                &traders,
                Load {
                    n_shards: n,
                    rounds,
                    salt: salt.fetch_add(1, Ordering::Relaxed) as u64,
                    shared_payer: false,
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
    for _ in 0..repeats {
        let o = run(
            &rpc,
            &bench,
            &shards,
            &traders,
            Load {
                n_shards: 8,
                rounds,
                salt: salt.fetch_add(1, Ordering::Relaxed) as u64,
                shared_payer: true,
            },
        );
        println!("{o:?}");
        results.push(o);
        let s0 = rpc.slot();
        while rpc.slot() < s0 + 3 {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    let configs: Vec<(usize, bool, &str)> = vec![
        (1, false, "1 shard"),
        (2, false, "2 shard-e"),
        (4, false, "4 shard-e"),
        (8, false, "8 shard-e"),
        (8, true, "8 shard-e, payer i përbashkët (kontroll negativ)"),
    ];
    let median = |mut v: Vec<usize>| -> usize {
        v.sort_unstable();
        v[v.len() / 2]
    };
    let base = median(
        results
            .iter()
            .filter(|o| o.shards == 1 && !o.shared_payer)
            .map(|o| o.max_per_slot)
            .collect(),
    );
    let mut md = String::from(
        "| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |\n|---|---:|---:|\n",
    );
    for &(n, shared, label) in &configs {
        let m = median(
            results
                .iter()
                .filter(|o| o.shards == n && o.shared_payer == shared)
                .map(|o| o.max_per_slot)
                .collect(),
        );
        md += &format!("| {label} | {m} | {:.2} |\n", m as f64 / base.max(1) as f64);
    }
    md += "\n| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n";
    for &(n, shared, label) in &configs {
        for (i, o) in results
            .iter()
            .filter(|o| o.shards == n && o.shared_payer == shared)
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
    print!("{md}");
    if let Some(path) = out {
        std::fs::write(path, md).unwrap();
    }
    // A failed swap means the benchmark itself is wrong (bad accounts or
    // state), not that the network was slow.
    assert!(results.iter().all(|o| o.failed == 0), "some swaps failed");
}
