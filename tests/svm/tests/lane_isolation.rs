//! Acceptance A-05 and A-09: shards are independent lanes.
//!
//! - A-09: in a split swap over all 8 shards, the accounts each swap writes
//!   (other than the trader's own) are disjoint between shards, and the shared
//!   accounts (market, registry, token program) are read-only everywhere.
//! - A-05: an operation on one shard leaves every other shard's state, vaults
//!   and tick arrays byte-identical, and interleaving work on two shards gives
//!   the same result as running each shard's work alone.

mod common;

use common::*;
use solana_signer::Signer;
use std::collections::HashSet;

fn lane_accounts(env: &Env, m: &Market, id: u8) -> Vec<Pubkey> {
    let a = shard_addrs(m, id, 0);
    let mut keys = vec![a.shard, a.vault_a, a.vault_b];
    let span = sharded_amm::state::array_span(env.market_state(m).params.tick_spacing);
    let mut start = -3 * span;
    while start <= 3 * span {
        let k = tick_array_pda(&a.shard, start);
        if env.raw(&k).is_some() {
            keys.push(k);
        }
        start += span;
    }
    keys
}

#[test]
fn split_swap_write_sets_are_disjoint() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000);
    let own: HashSet<Pubkey> = [t.kp.pubkey(), t.token_a, t.token_b].into();
    let shared: HashSet<Pubkey> = [m.market, m.registry, TOKEN_PROGRAM].into();
    let mut seen: HashSet<Pubkey> = HashSet::new();
    for id in 0..8u8 {
        let arrays = env.swap_arrays(&m, id, true);
        let ix = env.ix_swap(&m, &t, id, env.swap_args(true, 1_000), &arrays);
        let mut writes = HashSet::new();
        for meta in &ix.accounts {
            if shared.contains(&meta.pubkey) {
                assert!(!meta.is_writable, "shard {id}: {} writable", meta.pubkey);
            } else if meta.is_writable && !own.contains(&meta.pubkey) {
                writes.insert(meta.pubkey);
            }
        }
        // shard, two vaults and at least one tick array.
        assert!(writes.len() >= 4, "shard {id}: {writes:?}");
        assert!(
            seen.is_disjoint(&writes),
            "shard {id} shares a writable account"
        );
        seen.extend(writes);
    }
}

#[test]
fn work_on_one_shard_leaves_the_others_byte_identical() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000_000);
    let snapshot = |env: &Env| -> Vec<Vec<(Pubkey, Vec<u8>)>> {
        (0..8)
            .map(|id| {
                lane_accounts(env, &m, id)
                    .into_iter()
                    .map(|k| (k, env.raw(&k).unwrap().data))
                    .collect()
            })
            .collect()
    };
    let before = snapshot(&env);
    // Swaps both ways, a new position, a partial removal and a fee collect,
    // all on shard 3.
    env.swap(&m, &t, 3, true, 50_000_000).unwrap();
    env.swap(&m, &t, 3, false, 80_000_000).unwrap();
    env.add_liquidity(&m, &t, 3, -600, 600, 5_000_000_000)
        .unwrap();
    let i = env.ix_decrease(&m, &lp, 3, -1200, 1200, 1_000_000);
    let c = env.ix_collect(&m, &lp, 3, -1200, 1200);
    let kp = lp.kp.insecure_clone();
    env.send(&[i, c], &[&kp]).unwrap();
    let after = snapshot(&env);
    for id in 0..8 {
        if id == 3 {
            assert_ne!(before[id], after[id], "shard 3 must have changed");
        } else {
            assert_eq!(before[id], after[id], "shard {id} changed");
        }
    }
}

#[test]
fn interleaving_two_shards_equals_running_them_alone() {
    type Lane = (u128, i32, u128, [u64; 4], [u64; 4], u64, u64, u64, u64);
    let lane = |env: &Env, m: &Market, id: u8| -> Lane {
        let s = env.shard_state(m, id);
        let a = shard_addrs(m, id, 0);
        (
            s.sqrt_price_x64,
            s.tick_current,
            s.liquidity,
            s.fee_growth_global_a_x128,
            s.fee_growth_global_b_x128,
            s.protocol_fees_a,
            s.protocol_fees_b,
            env.balance(&a.vault_a),
            env.balance(&a.vault_b),
        )
    };
    let ops: Vec<(u8, bool, u64)> = (0..24u64)
        .map(|k| ((k % 2) as u8, k % 3 != 0, 3_000_000 + k * 7_777_777))
        .collect();
    // Interleaved: shard 0 and shard 1 alternate.
    let mut mixed = Env::new();
    let (m1, _) = mixed.open_seeded_market();
    let t1 = mixed.user(&m1, 1_000_000_000_000);
    for &(id, a_to_b, amount) in &ops {
        mixed.swap(&m1, &t1, id, a_to_b, amount).unwrap();
    }
    // Separate: all of shard 0's work, then all of shard 1's.
    let mut alone = Env::new();
    let (m2, _) = alone.open_seeded_market();
    let t2 = alone.user(&m2, 1_000_000_000_000);
    for lane_id in [0u8, 1] {
        for &(id, a_to_b, amount) in ops.iter().filter(|o| o.0 == lane_id) {
            alone.swap(&m2, &t2, id, a_to_b, amount).unwrap();
        }
    }
    for id in 0..8 {
        assert_eq!(lane(&mixed, &m1, id), lane(&alone, &m2, id), "shard {id}");
    }
}
