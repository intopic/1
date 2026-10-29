//! Rule 4 (2026-10-29): a shard becomes routable only with usable depth both
//! ways, not just high liquidity at the current price.
//!
//! - `verify_depth` runs two probe sales (A and B) on copies; a shard whose
//!   liquidity and vault balances meet the thresholds but whose depth is one
//!   narrow range fails it, and `open_market` refuses the market.
//! - Any liquidity change (decrease or increase) clears the mark, and a mark
//!   made before an applied fee change is not accepted; the market cannot
//!   open until the probe passes again.
//! - The probe writes only the shard's flag: tick arrays are passed
//!   read-only and stay byte-identical. Active shards cannot be probed.
//! - Expansion: `activate_shard` refuses an unverified shard.

mod common;

use common::*;
use sharded_amm::constants::REGISTRY_SEED;
use sharded_amm::errors::AmmError;
use sharded_amm::state::ShardStatus;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

/// Market whose readiness requires a 1e9 probe each way within 100 ticks.
fn strict_market(env: &mut Env) -> Market {
    let (mint_a, mint_b) = env.sorted_mints();
    let mut args = env.init_args(1, default_params());
    args.readiness.depth_probe_a = 1_000_000_000;
    args.readiness.depth_probe_b = 1_000_000_000;
    args.readiness.max_depth_impact_bps = 100;
    let admin = env.admin.insecure_clone();
    let i = env.ix_initialize_market(&admin.pubkey(), mint_a, mint_b, args);
    env.send(&[i], &[&admin]).expect("initialize_market");
    let market = market_pda(&mint_a, &mint_b, 1);
    Market {
        mint_a,
        mint_b,
        market_id: 1,
        market,
        registry: pda(&[REGISTRY_SEED, market.as_ref()]),
    }
}

/// Prepares and prices all 8; shard `narrow` gets one narrow range only.
fn seeded(env: &mut Env, m: &Market, narrow: Option<u8>) -> Lp {
    let keeper = env.keeper.insecure_clone();
    let lp = env.user(m, u64::MAX / 4);
    for id in 0..8 {
        env.prepare(m, &keeper, id).unwrap();
        env.send(
            &[Env::ix_initialize_price(m, &keeper.pubkey(), id, Q64)],
            &[&keeper],
        )
        .unwrap();
        if Some(id) == narrow {
            // Plenty of L at the price and both vaults funded, but only
            // +-60 ticks deep: ~3e7 of either token empties it.
            env.add_liquidity(m, &lp, id, -60, 60, 10_000_000_000)
                .unwrap();
        } else {
            env.add_liquidity(m, &lp, id, -1200, 1200, 1_000_000_000_000)
                .unwrap();
        }
    }
    lp
}

fn open(env: &mut Env, m: &Market) -> Result<u64, TransactionError> {
    let keeper = env.keeper.insecure_clone();
    let i = env.ix_open_market(m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
    env.send(&[i], &[&keeper])
}

#[test]
fn narrow_liquidity_fails_the_probe_and_blocks_opening() {
    let mut env = Env::new();
    let m = strict_market(&mut env);
    seeded(&mut env, &m, Some(3));
    let s3 = env.shard_state(&m, 3);
    let r = env.market_state(&m).readiness;
    assert!(s3.liquidity >= r.min_liquidity, "old readiness would pass");
    let a = shard_addrs(&m, 3, 0);
    assert!(
        env.balance(&a.vault_a) >= r.min_reserve_a && env.balance(&a.vault_b) >= r.min_reserve_b
    );
    assert!(!s3.depth_verified);
    expect_custom(
        env.verify_depth(&m, 3),
        amm_code(AmmError::InsufficientDepth),
    );
    for id in (0..8).filter(|i| *i != 3) {
        assert!(env.shard_state(&m, id).depth_verified, "shard {id}");
    }
    expect_custom(
        open(&mut env, &m),
        amm_code(AmmError::InitialShardsNotReady),
    );
}

#[test]
fn probe_limit_is_enforced() {
    // Same narrow shard, but a probe small enough to fill within 100 ticks.
    let mut env = Env::new();
    let (mint_a, mint_b) = env.sorted_mints();
    let mut args = env.init_args(1, default_params());
    args.readiness.depth_probe_a = 1_000_000;
    args.readiness.depth_probe_b = 1_000_000;
    args.readiness.max_depth_impact_bps = 100;
    let admin = env.admin.insecure_clone();
    let i = env.ix_initialize_market(&admin.pubkey(), mint_a, mint_b, args);
    env.send(&[i], &[&admin]).unwrap();
    let market = market_pda(&mint_a, &mint_b, 1);
    let m = Market {
        mint_a,
        mint_b,
        market_id: 1,
        market,
        registry: pda(&[REGISTRY_SEED, market.as_ref()]),
    };
    seeded(&mut env, &m, Some(3));
    assert!(env.shard_state(&m, 3).depth_verified);
    open(&mut env, &m).unwrap();
}

#[test]
fn decrease_clears_the_mark_until_verified_again() {
    let mut env = Env::new();
    let m = strict_market(&mut env);
    let lp = seeded(&mut env, &m, None);
    assert!(env.shard_state(&m, 6).depth_verified);
    let i = env.ix_decrease(&m, &lp, 6, -1200, 1200, 1);
    let kp = lp.kp.insecure_clone();
    env.send(&[i], &[&kp]).unwrap();
    assert!(!env.shard_state(&m, 6).depth_verified, "decrease clears it");
    expect_custom(
        open(&mut env, &m),
        amm_code(AmmError::InitialShardsNotReady),
    );
    // Read-only arrays: the probe leaves them byte-identical.
    let arrays: Vec<Pubkey> = env
        .swap_arrays(&m, 6, true)
        .into_iter()
        .chain(env.swap_arrays(&m, 6, false))
        .collect();
    let before: Vec<Vec<u8>> = arrays.iter().map(|k| env.raw(k).unwrap().data).collect();
    env.verify_depth(&m, 6).unwrap();
    let after: Vec<Vec<u8>> = arrays.iter().map(|k| env.raw(k).unwrap().data).collect();
    assert_eq!(before, after);
    assert!(env.shard_state(&m, 6).depth_verified);
    open(&mut env, &m).unwrap();
    assert_eq!(env.shard_state(&m, 6).status, ShardStatus::Active);
    // Active shards are not probed again.
    expect_custom(
        env.verify_depth(&m, 6),
        amm_code(AmmError::ShardAlreadyActive),
    );
}

#[test]
fn expansion_shard_needs_the_probe() {
    let mut env = Env::new();
    let m = strict_market(&mut env);
    let lp = seeded(&mut env, &m, None);
    open(&mut env, &m).unwrap();
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 8).unwrap();
    env.send(
        &[Env::ix_initialize_price(&m, &keeper.pubkey(), 8, Q64)],
        &[&keeper],
    )
    .unwrap();
    // Narrow first: the automatic probe fails and activation is refused.
    env.add_liquidity(&m, &lp, 8, -60, 60, 10_000_000_000)
        .unwrap();
    assert!(!env.shard_state(&m, 8).depth_verified);
    let i = Env::ix_activate(&m, &keeper.pubkey(), 8, 0);
    let res = env.send(&[i], &[&keeper]);
    assert!(res.is_err(), "{res:?}");
    assert_eq!(env.registry(&m).active_mask, 0xff);
    // Wide liquidity: the probe passes and the shard activates.
    env.add_liquidity(&m, &lp, 8, -1200, 1200, 1_000_000_000_000)
        .unwrap();
    assert!(env.shard_state(&m, 8).depth_verified);
    let i = Env::ix_activate(&m, &keeper.pubkey(), 8, 0);
    env.send(&[i], &[&keeper]).unwrap();
    assert_eq!(env.registry(&m).active_mask, 0x1ff);
}

/// Re-audit P2-5: a depth mark made before a fee change is not accepted
/// after it, and an increase of liquidity clears the mark like a decrease.
#[test]
fn fee_change_and_increase_invalidate_the_mark() {
    let mut env = Env::new();
    let m = strict_market(&mut env);
    let lp = seeded(&mut env, &m, None);
    assert!((0..8).all(|id| env.shard_state(&m, id).depth_verified));
    let v0 = env.market_state(&m).config_version;
    let admin = env.admin.insecure_clone();
    env.send(
        &[Env::ix_update_fees(&m, &admin.pubkey(), 1_000, 2_000)],
        &[&admin],
    )
    .unwrap();
    env.warp(sharded_amm::constants::FEE_TIMELOCK_SECONDS);
    env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin])
        .unwrap();
    assert_eq!(env.market_state(&m).config_version, v0 + 1);
    // The marks are still set but were made under the old fee.
    assert!(env.shard_state(&m, 0).depth_verified);
    expect_custom(
        open(&mut env, &m),
        amm_code(AmmError::InitialShardsNotReady),
    );
    for id in 0..8 {
        env.verify_depth(&m, id).unwrap();
        assert_eq!(env.shard_state(&m, id).depth_config_version, v0 + 1);
    }
    // An increase (sent directly, without the helper's re-probe) clears it.
    let i = env.ix_increase(&m, &lp, 5, -1200, 1200, 1_000);
    let kp = lp.kp.insecure_clone();
    env.send(&[i], &[&kp]).unwrap();
    assert!(!env.shard_state(&m, 5).depth_verified, "increase clears it");
    expect_custom(
        open(&mut env, &m),
        amm_code(AmmError::InitialShardsNotReady),
    );
    env.verify_depth(&m, 5).unwrap();
    open(&mut env, &m).unwrap();
}
