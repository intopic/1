//! Validator benchmark, step 1 (acceptance A-10): builds one open market per
//! configuration (1, 2, 4, 8 lanes), each with the same total liquidity, and
//! funded traders in LiteSVM with the real SBF program,
//! then writes every account as a `solana-test-validator --account-dir` JSON
//! file plus `bench.json` (addresses, tick arrays, trader keys, and the four
//! rebalancer keys listed on the 8-lane market for `examples/rebalancer.rs`).
//!
//! The keys are throwaway keypairs for a local test validator only.
//!
//! Usage: cargo run --release --example bench_setup -- <out-dir> [traders]

#[path = "../tests/common/mod.rs"]
mod common;

use base64::Engine;
use common::*;
use serde_json::json;
use sharded_amm::math::tick_math::sqrt_price_at_tick;
use solana_signer::Signer;

/// Price sits in the middle of the tick array starting at 0 (spacing 60,
/// span 1920), so small swaps never leave it and every swap can list the
/// same arrays.
const START_TICK: i32 = 960;
/// Liquidity of every configuration, split evenly over its active lanes.
const TOTAL_LIQUIDITY: u128 = 1_000_000_000_000_000;
/// Shards a configuration does not use: just above the readiness minimum.
const IDLE_LIQUIDITY: u128 = 1_000_000_000;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("out dir"));
    let traders: usize = args.next().map(|v| v.parse().unwrap()).unwrap_or(256);
    let accounts_dir = out.join("accounts");
    std::fs::create_dir_all(&accounts_dir).unwrap();

    let mut env = Env::new();
    let keeper = env.keeper.insecure_clone();
    let (mint_a, mint_b) = env.sorted_mints();
    let sqrt = sqrt_price_at_tick(START_TICK).unwrap();
    let mut keys = vec![mint_a, mint_b];
    let mut markets = Vec::new();
    let mut lp = None;
    let mut first = None;
    // One market per configuration, all with the same total liquidity
    // (equal capital): `n` shards share TOTAL_LIQUIDITY, the other 8 - n
    // hold only the readiness minimum so the market can open.
    for n in [1u8, 2, 4, 8] {
        let m = env.market_with(mint_a, mint_b, u64::from(n));
        if lp.is_none() {
            lp = Some(env.user(&m, u64::MAX / 4));
        }
        let lp = lp.as_ref().unwrap();
        for id in 0..8 {
            env.prepare(&m, &keeper, id).unwrap();
            env.send(
                &[Env::ix_initialize_price(&m, &keeper.pubkey(), id, sqrt)],
                &[&keeper],
            )
            .unwrap();
            let l = if id < n {
                TOTAL_LIQUIDITY / u128::from(n)
            } else {
                IDLE_LIQUIDITY
            };
            env.add_liquidity(&m, lp, id, -3840, 5760, l).unwrap();
        }
        let open = env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
        env.send(&[open], &[&keeper]).unwrap();
        // The 8-lane market lists four rebalancer keys for the rebalancer
        // bot (`examples/rebalancer.rs`); each pays its own fees and holds
        // no tokens, since a rebalance needs no capital.
        let mut rebalancers = Vec::new();
        if n == 8 {
            let rs: Vec<_> = (0..4).map(|_| env.user(&m, 0)).collect();
            env.set_rebalancers(&m, &rs.iter().map(|r| r.kp.pubkey()).collect::<Vec<_>>());
            for r in &rs {
                keys.extend([r.kp.pubkey(), r.token_a, r.token_b]);
                rebalancers.push(json!({
                    "secret": r.kp.to_bytes().to_vec(),
                    "token_a": r.token_a.to_string(),
                    "token_b": r.token_b.to_string(),
                }));
            }
        }

        let spacing = env.market_state(&m).params.tick_spacing;
        let span = sharded_amm::state::array_span(spacing);
        keys.extend([m.market, m.registry]);
        let mut shards = Vec::new();
        for id in 0..n {
            let a = shard_addrs(&m, id, 0);
            let mut start = -2 * span;
            while start <= 3 * span {
                let k = tick_array_pda(&a.shard, start);
                if env.raw(&k).is_some() {
                    keys.push(k);
                }
                start += span;
            }
            let pick = |starts: [i32; 2]| -> Vec<String> {
                starts
                    .iter()
                    .map(|s| tick_array_pda(&a.shard, *s).to_string())
                    .collect()
            };
            shards.push(json!({
                "id": id,
                "shard": a.shard.to_string(),
                "vault_a": a.vault_a.to_string(),
                "vault_b": a.vault_b.to_string(),
                "vault_authority": a.vault_authority.to_string(),
                "arrays_a_to_b": pick([0, -span]),
                "arrays_b_to_a": pick([0, span]),
            }));
            keys.extend([a.shard, a.vault_a, a.vault_b]);
            assert_eq!(env.shard_state(&m, id).tick_current, START_TICK);
        }
        markets.push(json!({
            "n_shards": n,
            "market": m.market.to_string(),
            "registry": m.registry.to_string(),
            "total_liquidity": TOTAL_LIQUIDITY.to_string(),
            "shards": shards,
            "rebalancers": rebalancers,
        }));
        first.get_or_insert(m);
    }
    let m = first.unwrap();

    let mut trader_rows = Vec::new();
    for _ in 0..traders {
        let t = env.user(&m, 1_000_000_000_000);
        keys.extend([t.kp.pubkey(), t.token_a, t.token_b]);
        trader_rows.push(json!({
            "secret": t.kp.to_bytes().to_vec(),
            "token_a": t.token_a.to_string(),
            "token_b": t.token_b.to_string(),
        }));
    }

    let b64 = base64::engine::general_purpose::STANDARD;
    for k in &keys {
        let acc = env.raw(k).unwrap_or_else(|| panic!("{k} missing"));
        let doc = json!({
            "pubkey": k.to_string(),
            "account": {
                "lamports": acc.lamports,
                "data": [b64.encode(&acc.data), "base64"],
                "owner": acc.owner.to_string(),
                "executable": false,
                "rentEpoch": 0,
                "space": acc.data.len(),
            }
        });
        std::fs::write(
            accounts_dir.join(format!("{k}.json")),
            serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
    }
    let bench = json!({
        "program_id": sharded_amm::ID.to_string(),
        "mint_a": mint_a.to_string(),
        "mint_b": mint_b.to_string(),
        "start_tick": START_TICK,
        "markets": markets,
        "traders": trader_rows,
    });
    std::fs::write(out.join("bench.json"), serde_json::to_vec(&bench).unwrap()).unwrap();
    println!(
        "wrote {} accounts and {} traders to {}",
        keys.len(),
        traders,
        out.display()
    );
}
