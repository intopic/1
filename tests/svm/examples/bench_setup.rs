//! Validator benchmark, step 1 (acceptance A-10): builds an open market with
//! 8 seeded shards and funded traders in LiteSVM with the real SBF program,
//! then writes every account as a `solana-test-validator --account-dir` JSON
//! file plus `bench.json` (addresses, tick arrays, trader keys).
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

fn main() {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("out dir"));
    let traders: usize = args.next().map(|v| v.parse().unwrap()).unwrap_or(256);
    let accounts_dir = out.join("accounts");
    std::fs::create_dir_all(&accounts_dir).unwrap();

    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    let lp = env.user(&m, u64::MAX / 4);
    let sqrt = sqrt_price_at_tick(START_TICK).unwrap();
    for id in 0..8 {
        env.prepare(&m, &keeper, id).unwrap();
        env.send(
            &[Env::ix_initialize_price(&m, &keeper.pubkey(), id, sqrt)],
            &[&keeper],
        )
        .unwrap();
        env.add_liquidity(&m, &lp, id, -3840, 5760, 1_000_000_000_000_000)
            .unwrap();
    }
    let open = env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
    env.send(&[open], &[&keeper]).unwrap();

    let spacing = env.market_state(&m).params.tick_spacing;
    let span = sharded_amm::state::array_span(spacing);
    let mut keys = vec![m.mint_a, m.mint_b, m.market, m.registry];
    let mut shards = Vec::new();
    for id in 0..8u8 {
        let a = shard_addrs(&m, id, 0);
        let mut arrays = Vec::new();
        let mut start = -2 * span;
        while start <= 3 * span {
            let k = tick_array_pda(&a.shard, start);
            if env.raw(&k).is_some() {
                arrays.push((start, k));
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
        "market": m.market.to_string(),
        "registry": m.registry.to_string(),
        "mint_a": m.mint_a.to_string(),
        "mint_b": m.mint_b.to_string(),
        "start_tick": START_TICK,
        "shards": shards,
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
