//! ENG-07: growing an open market beyond the 8 initial shards.
//!
//! A new shard is prepared, priced, funded and only then activated by the
//! keeper against an active reference shard. Activation fails unless every
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
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), id, 0)], &[&keeper])
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
            env.send(&[Env::ix_activate(&m, &keeper.pubkey(), id, 0)], &[&keeper]),
            amm_code(AmmError::ShardAlreadyActive),
        );
    }
    // The new shard can serve as a reference for the next one.
    stage(&mut env, &m, &lp, 30, Q64, 1_000_000_000_000);
    env.send(
        &[Env::ix_activate(&m, &keeper.pubkey(), 30, 55)],
        &[&keeper],
    )
    .unwrap();
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
        env.send(&[Env::ix_activate(&m, &admin.pubkey(), 9, 0)], &[&admin]),
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
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 10, 0)], &[&keeper]),
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
    env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 11, 0)], &[&keeper])
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
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 12, 0)], &[&keeper]),
        amm_code(AmmError::PriceDeviationTooHigh),
    );

    // Unfunded, or funded below the readiness minimum.
    stage(&mut env, &m, &lp, 13, Q64, 0);
    expect_custom(
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 13, 0)], &[&keeper]),
        amm_code(AmmError::ShardNotReady),
    );
    stage(&mut env, &m, &lp, 14, Q64, 10);
    expect_custom(
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 14, 0)], &[&keeper]),
        amm_code(AmmError::ShardNotReady),
    );
    // Prepared but never priced.
    env.prepare(&m, &keeper, 15).unwrap();
    expect_err(env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 15, 0)], &[&keeper]));

    // The reference must itself be active.
    expect_custom(
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 9, 13)], &[&keeper]),
        amm_code(AmmError::ShardNotActive),
    );
    // Wrong vaults for the shard.
    let mut i = Env::ix_activate(&m, &keeper.pubkey(), 9, 0);
    i.accounts[4].pubkey = shard_addrs(&m, 0, 0).vault_a;
    expect_custom(env.send(&[i], &[&keeper]), anchor_codes::CONSTRAINT_HAS_ONE);
    // An active reference shard of another open market.
    let (other, _) = env.open_seeded_market();
    let mut i = Env::ix_activate(&m, &keeper.pubkey(), 9, 0);
    i.accounts[6].pubkey = shard_addrs(&other, 0, 0).shard;
    expect_custom(env.send(&[i], &[&keeper]), anchor_codes::CONSTRAINT_HAS_ONE);

    // Paused market: nothing activates; after unpause it does.
    let pauser = env.pauser.insecure_clone();
    env.send(
        &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
        &[&pauser],
    )
    .unwrap();
    assert_eq!(env.market_state(&m).status, MarketStatus::Paused);
    expect_custom(
        env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 9, 0)], &[&keeper]),
        amm_code(AmmError::InvalidMarketStatus),
    );
    expect_custom(
        env.prepare(&m, &keeper, 20),
        amm_code(AmmError::InvalidMarketStatus),
    );
    env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), false)], &[&admin])
        .unwrap();
    env.send(&[Env::ix_activate(&m, &keeper.pubkey(), 9, 0)], &[&keeper])
        .unwrap();

    // IDs at and above the capacity of 56 never prepare.
    expect_err(env.prepare(&m, &keeper, 56));
    expect_err(env.prepare(&m, &keeper, 255));
    let reg = env.registry(&m);
    assert_eq!(reg.active_mask, 0xff | (1 << 9) | (1 << 11));
    assert_eq!(reg.active_mask >> 56, 0);
}

#[test]
fn all_56_shards_can_be_active() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    for id in 8u8..56 {
        stage(&mut env, &m, &lp, id, Q64, 1_000_000_000);
        let reference = id - 1 - (id % 7);
        env.send(
            &[Env::ix_activate(&m, &keeper.pubkey(), id, reference)],
            &[&keeper],
        )
        .unwrap();
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
