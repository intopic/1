//! ENG-07: growing an open market beyond the 8 initial shards.
//!
//! A new shard is prepared, priced, funded and only then activated by the
//! keeper against the median of every open shard (D-09). Activation fails unless every
//! rule holds; after it, the shard trades like the initial ones.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::math::tick_math::sqrt_price_at_tick;
use sharded_amm::state::{MarketStatus, ShardStatus};
use solana_signer::Signer;

/// Prepares, prices and (optionally) funds shard `id` on an open market.
fn stage(env: &mut Env, m: &Market, lp: &Lp, id: u8, sqrt_price: u128, liquidity: u128) {
    let keeper = env.keeper.insecure_clone();
    env.prepare(m, &keeper, id).expect("prepare");
    env.send(
        &[Env::ix_initialize_price(
            m,
            &keeper.pubkey(),
            id,
            sqrt_price,
        )],
        &[&keeper],
    )
    .expect("price");
    if liquidity > 0 {
        let tick = env.shard_state(m, id).tick_current;
        let c = tick.div_euclid(60) * 60;
        env.add_liquidity(m, lp, id, c - 1200, c + 1260, liquidity)
            .expect("fund");
    }
}

#[test]
fn shards_8_and_55_activate_and_trade() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    for id in [8u8, 55] {
        stage(&mut env, &m, &lp, id, Q64, 1_000_000_000_000);
        assert_eq!(env.shard_state(&m, id).status, ShardStatus::Prepared);
        // Not yet active: no swaps.
        let t = env.user(&m, 1_000_000_000);
        expect_custom(
            env.swap(&m, &t, id, true, 1_000),
            amm_code(AmmError::ShardNotActive),
        );
        let before = env.registry(&m);
        env.send(
            &[env.ix_activate_open(&m, &keeper.pubkey(), id)],
            &[&keeper],
        )
        .unwrap();
        let after = env.registry(&m);
        assert_eq!(after.active_mask, before.active_mask | (1u64 << id));
        assert_eq!(after.registry_version, before.registry_version + 1);
        assert_eq!(env.shard_state(&m, id).status, ShardStatus::Active);
        // Trades in both directions and pays out.
        let b0 = env.balance(&t.token_b);
        env.swap(&m, &t, id, true, 1_000_000).unwrap();
        assert!(env.balance(&t.token_b) > b0);
        env.swap(&m, &t, id, false, 1_000_000).unwrap();
        // Activating twice fails.
        expect_custom(
            env.send(
                &[env.ix_activate_open(&m, &keeper.pubkey(), id)],
                &[&keeper],
            ),
            amm_code(AmmError::ShardAlreadyActive),
        );
    }
    // The new shards are references for the next one.
    stage(&mut env, &m, &lp, 30, Q64, 1_000_000_000_000);
    assert_eq!(env.open_others(&m, 30).len(), 10);
    let i = env.ix_activate_open(&m, &keeper.pubkey(), 30);
    env.send(&[i], &[&keeper]).unwrap();
    assert_eq!(
        env.registry(&m).active_mask,
        0xff | (1 << 8) | (1 << 30) | (1 << 55)
    );
}

#[test]
fn activation_rules() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    let admin = env.admin.insecure_clone();

    // Only the keeper.
    stage(&mut env, &m, &lp, 9, Q64, 1_000_000_000_000);
    expect_custom(
        env.send(&[env.ix_activate_open(&m, &admin.pubkey(), 9)], &[&admin]),
        amm_code(AmmError::Unauthorized),
    );

    // Price more than 100 bps away from the reference: tick 200 is ~2.02%.
    stage(
        &mut env,
        &m,
        &lp,
        10,
        sqrt_price_at_tick(200).unwrap(),
        1_000_000_000_000,
    );
    expect_custom(
        env.send(
            &[env.ix_activate_open(&m, &keeper.pubkey(), 10)],
            &[&keeper],
        ),
        amm_code(AmmError::PriceDeviationTooHigh),
    );
    // Just inside 100 bps (tick 99 is ~0.995% in price) passes.
    stage(
        &mut env,
        &m,
        &lp,
        11,
        sqrt_price_at_tick(99).unwrap(),
        1_000_000_000_000,
    );
    env.send(
        &[env.ix_activate_open(&m, &keeper.pubkey(), 11)],
        &[&keeper],
    )
    .unwrap();
    // Below the reference too.
    stage(
        &mut env,
        &m,
        &lp,
        12,
        sqrt_price_at_tick(-101).unwrap(),
        1_000_000_000_000,
    );
    expect_custom(
        env.send(
            &[env.ix_activate_open(&m, &keeper.pubkey(), 12)],
            &[&keeper],
        ),
        amm_code(AmmError::PriceDeviationTooHigh),
    );

    // Unfunded, or funded below the readiness minimum.
    stage(&mut env, &m, &lp, 13, Q64, 0);
    expect_custom(
        env.send(
            &[env.ix_activate_open(&m, &keeper.pubkey(), 13)],
            &[&keeper],
        ),
        amm_code(AmmError::ShardNotReady),
    );
    stage(&mut env, &m, &lp, 14, Q64, 10);
    expect_custom(
        env.send(
            &[env.ix_activate_open(&m, &keeper.pubkey(), 14)],
            &[&keeper],
        ),
        amm_code(AmmError::ShardNotReady),
    );
    // Prepared but never priced.
    env.prepare(&m, &keeper, 15).unwrap();
    expect_err(env.send(
        &[env.ix_activate_open(&m, &keeper.pubkey(), 15)],
        &[&keeper],
    ));

    // D-09: the references are exactly the open shards. A subset, a
    // duplicate, a shard that is not active, or a shard of another market
    // is refused, so the caller cannot pick convenient references.
    let all = env.open_others(&m, 9);
    assert_eq!(all, [0, 1, 2, 3, 4, 5, 6, 7, 11]);
    for (refs, err) in [
        (&all[..8], AmmError::InvalidAccountCount),
        (&[0, 1, 2, 3, 4, 5, 6, 7, 7][..], AmmError::DuplicateAccount),
        (&[0, 1, 2, 3, 4, 5, 6, 7, 13][..], AmmError::ShardMismatch),
    ] {
        expect_custom(
            env.send(
                &[Env::ix_activate(&m, &keeper.pubkey(), 9, refs)],
                &[&keeper],
            ),
            amm_code(err),
        );
    }
    // Wrong vaults for the shard.
    let mut i = env.ix_activate_open(&m, &keeper.pubkey(), 9);
    i.accounts[5].pubkey = shard_addrs(&m, 0, 0).vault_a;
    expect_custom(env.send(&[i], &[&keeper]), anchor_codes::CONSTRAINT_HAS_ONE);
    // An active shard of another open market in place of one of ours.
    let (other, _) = env.open_seeded_market();
    let mut i = env.ix_activate_open(&m, &keeper.pubkey(), 9);
    i.accounts[7].pubkey = shard_addrs(&other, 0, 0).shard;
    expect_custom(
        env.send(&[i], &[&keeper]),
        amm_code(AmmError::ShardMismatch),
    );
    // A swap-paused shard is not a reference; with it paused the open set
    // shrinks and the old full list is refused.
    let pauser = env.pauser.insecure_clone();
    env.send(
        &[ix_shard_flags(
            &m,
            &pauser.pubkey(),
            11,
            sharded_amm::constants::PAUSE_SWAPS,
        )],
        &[&pauser],
    )
    .unwrap();
    expect_custom(
        env.send(
            &[Env::ix_activate(&m, &keeper.pubkey(), 9, &all)],
            &[&keeper],
        ),
        amm_code(AmmError::InvalidAccountCount),
    );

    // Paused market: nothing activates; after unpause it does.
    env.send(
        &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
        &[&pauser],
    )
    .unwrap();
    assert_eq!(env.market_state(&m).status, MarketStatus::Paused);
    expect_custom(
        env.send(&[env.ix_activate_open(&m, &keeper.pubkey(), 9)], &[&keeper]),
        amm_code(AmmError::InvalidMarketStatus),
    );
    expect_custom(
        env.prepare(&m, &keeper, 20),
        amm_code(AmmError::InvalidMarketStatus),
    );
    env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), false)], &[&admin])
        .unwrap();
    env.send(&[env.ix_activate_open(&m, &keeper.pubkey(), 9)], &[&keeper])
        .unwrap();

    // IDs at and above the capacity of 56 never prepare.
    expect_err(env.prepare(&m, &keeper, 56));
    expect_err(env.prepare(&m, &keeper, 255));
    let reg = env.registry(&m);
    assert_eq!(reg.active_mask, 0xff | (1 << 9) | (1 << 11));
    assert!(env.shard_state(&m, 11).swaps_paused());
    assert_eq!(reg.active_mask >> 56, 0);
}

#[test]
fn all_56_shards_can_be_active() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    for id in 8u8..56 {
        stage(&mut env, &m, &lp, id, Q64, 1_000_000_000);
        let i = env.ix_activate_open(&m, &keeper.pubkey(), id);
        env.send(&[i], &[&keeper]).unwrap();
    }
    let reg = env.registry(&m);
    assert_eq!(reg.active_mask, (1u64 << 56) - 1);
    let t = env.user(&m, 1_000_000_000_000);
    for id in 0u8..56 {
        env.swap(&m, &t, id, id % 2 == 0, 100_000).unwrap();
    }
    // Full: no further shard ID exists.
    expect_err(env.prepare(&m, &keeper, 56));
}

/// Re-audit P2-1 at capacity, with D-09: with 54 of 56 shards swap-paused
/// the two trading ones are the references (exactly those two); with all 56
/// paused a bootstrap against all 55 others reopens one, and then the rest
/// against it.
#[test]
fn reopen_at_capacity_with_few_or_no_trading_shards() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    for id in 8u8..56 {
        stage(&mut env, &m, &lp, id, Q64, 1_000_000_000);
        let i = env.ix_activate_open(&m, &keeper.pubkey(), id);
        env.send(&[i], &[&keeper]).unwrap();
    }
    let (admin, pauser) = (env.admin.insecure_clone(), env.pauser.insecure_clone());
    let flags = sharded_amm::constants::PAUSE_SWAPS;
    for id in 0u8..54 {
        env.send(
            &[ix_shard_flags(&m, &pauser.pubkey(), id, flags)],
            &[&pauser],
        )
        .unwrap();
    }
    let reopen = |env: &mut Env, id: u8, refs: &[u8]| {
        env.send(&[ix_reopen(&m, &admin.pubkey(), id, 0, refs)], &[&admin])
    };
    expect_custom(
        reopen(&mut env, 10, &[54]),
        amm_code(AmmError::InvalidAccountCount),
    );
    // A paused shard in place of a trading one is not accepted.
    expect_custom(
        reopen(&mut env, 10, &[54, 3]),
        amm_code(AmmError::ShardMismatch),
    );
    reopen(&mut env, 10, &[54, 55]).unwrap();
    for id in [10u8, 54, 55] {
        env.send(
            &[ix_shard_flags(&m, &pauser.pubkey(), id, flags)],
            &[&pauser],
        )
        .unwrap();
    }
    assert_eq!(env.registry(&m).swap_paused_mask, (1u64 << 56) - 1);
    let others: Vec<u8> = (0u8..56).filter(|&i| i != 20).collect();
    expect_custom(
        reopen(&mut env, 20, &[21, 22, 23]),
        amm_code(AmmError::InvalidAccountCount),
    );
    reopen(&mut env, 20, &others).unwrap();
    reopen(&mut env, 21, &[20]).unwrap();
    let t = env.user(&m, 1_000_000_000);
    env.swap(&m, &t, 21, true, 100_000).unwrap();
}
