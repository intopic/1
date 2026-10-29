//! Anti-JIT (security review): liquidity added in a slot cannot leave, and
//! its fees cannot be collected, in that same slot. A position records
//! `last_liquidity_slot` when it opens and on every increase;
//! `decrease_liquidity` and `collect_fees` need a later slot. From the next
//! slot the exit is free again, and a D-16 transfer keeps the field, so a
//! transfer cannot be used to skip the wait.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::instruction as ixs;
use solana_signer::Signer;

const RANGE: (i32, i32) = (-600, 600);

fn slot(env: &Env) -> u64 {
    env.svm.get_sysvar::<anchor_lang::prelude::Clock>().slot
}

#[test]
fn liquidity_cannot_leave_in_the_slot_it_arrived() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let (lo, hi) = RANGE;
    let jit = env.user(&m, 1_000_000_000_000);
    let trader = env.user(&m, 1_000_000_000_000);
    env.ensure_arrays(&m, 0, lo, hi);
    let shard = shard_addrs(&m, 0, 0).shard;
    let pos = position_pda(&shard, &jit.kp.pubkey(), lo, hi);
    let kp = jit.kp.insecure_clone();

    // Open, add and remove in one transaction: refused.
    let open = Env::ix_open_position(&m, &jit.kp.pubkey(), &shard, lo, hi);
    let add = env.ix_increase(&m, &jit, 0, lo, hi, 1_000_000_000);
    let remove = env.ix_decrease(&m, &jit, 0, lo, hi, 1_000_000_000);
    expect_custom(
        env.send(&[open.clone(), add.clone(), remove.clone()], &[&kp]),
        amm_code(AmmError::LiquidityTooRecent),
    );
    // Add, let a swap through, collect in the same slot: refused too.
    env.send(&[open, add.clone()], &[&kp]).unwrap();
    let s = slot(&env);
    assert_eq!(env.position(&pos).last_liquidity_slot, s);
    env.swap(&m, &trader, 0, true, 50_000_000).unwrap();
    let collect = env.ix_collect(&m, &jit, 0, lo, hi);
    expect_custom(
        env.send(std::slice::from_ref(&collect), &[&kp]),
        amm_code(AmmError::LiquidityTooRecent),
    );
    expect_custom(
        env.send(std::slice::from_ref(&remove), &[&kp]),
        amm_code(AmmError::LiquidityTooRecent),
    );

    // The next slot: the exit is free, in full, fees included.
    env.svm.warp_to_slot(s + 1);
    env.send(&[remove.clone(), collect.clone()], &[&kp])
        .unwrap();
    let p = env.position(&pos);
    assert_eq!((p.liquidity, p.tokens_owed_a, p.tokens_owed_b), (0, 0, 0));
    assert_eq!(p.last_liquidity_slot, s, "an exit does not move the mark");

    // A later increase moves the mark and the wait starts again.
    env.svm.warp_to_slot(s + 5);
    env.send(&[add], &[&kp]).unwrap();
    assert_eq!(env.position(&pos).last_liquidity_slot, s + 5);
    expect_custom(
        env.send(std::slice::from_ref(&remove), &[&kp]),
        amm_code(AmmError::LiquidityTooRecent),
    );
    env.svm.warp_to_slot(s + 6);
    env.send(&[remove], &[&kp]).unwrap();
}

#[test]
fn a_transfer_does_not_reset_the_wait() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let (lo, hi) = RANGE;
    let alice = env.user(&m, 1_000_000_000_000);
    let bob = env.user(&m, 0);
    env.ensure_arrays(&m, 0, lo, hi);
    let shard = shard_addrs(&m, 0, 0).shard;
    let pos = position_pda(&shard, &alice.kp.pubkey(), lo, hi);
    let open = Env::ix_open_position(&m, &alice.kp.pubkey(), &shard, lo, hi);
    let add = env.ix_increase(&m, &alice, 0, lo, hi, 1_000_000);
    let akp = alice.kp.insecure_clone();
    env.send(&[open, add], &[&akp]).unwrap();
    let s = slot(&env);
    let propose = ix(
        sharded_amm::accounts::ProposePositionTransfer {
            owner: alice.kp.pubkey(),
            position: pos,
        },
        ixs::ProposePositionTransfer {
            new_owner: bob.kp.pubkey(),
        },
    );
    let accept = ix(
        sharded_amm::accounts::AcceptPositionTransfer {
            new_owner: bob.kp.pubkey(),
            position: pos,
        },
        ixs::AcceptPositionTransfer {},
    );
    env.send(&[propose], &[&akp]).unwrap();
    let bkp = bob.kp.insecure_clone();
    env.send(&[accept], &[&bkp]).unwrap();
    assert_eq!(env.position(&pos).last_liquidity_slot, s);
    // Bob's exit in the same slot is refused exactly like alice's would be.
    let mut remove = env.ix_decrease(&m, &alice, 0, lo, hi, 1_000_000);
    remove.accounts[0].pubkey = bob.kp.pubkey();
    remove.accounts[6].pubkey = bob.token_a;
    remove.accounts[7].pubkey = bob.token_b;
    expect_custom(
        env.send(std::slice::from_ref(&remove), &[&bkp]),
        amm_code(AmmError::LiquidityTooRecent),
    );
    env.svm.warp_to_slot(s + 1);
    env.send(&[remove], &[&bkp]).unwrap();
}
