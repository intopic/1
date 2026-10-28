//! ENG-04: price, tick arrays, positions, opening and cancelling a market.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::state::{MarketStatus, ShardStatus};
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn full_setup_opens_all_eight_atomically() {
    let mut env = Env::new();
    let m = env.market();
    let lp = env.seed_initial_shards(&m, 1_000_000_000_000);
    let keeper = env.keeper.insecure_clone();
    let cu = env
        .send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>())],
            &[&keeper],
        )
        .unwrap();
    println!("open_market CU: {cu}");
    let r = env.registry(&m);
    assert_eq!((r.prepared_mask, r.active_mask), (0xff, 0xff));
    assert_eq!(env.market_state(&m).status, MarketStatus::Open);
    for id in 0..8 {
        let s = env.shard_state(&m, id);
        assert_eq!(s.status, ShardStatus::Active);
        assert_eq!(s.liquidity, 1_000_000_000_000);
        let a = shard_addrs(&m, id, 0);
        assert!(env.balance(&a.vault_a) > 0 && env.balance(&a.vault_b) > 0);
    }
    // Opening twice fails.
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>())],
            &[&keeper],
        ),
        amm_code(AmmError::ShardMismatch),
    );
    let _ = lp;
}

#[test]
fn open_market_requires_every_shard_ready() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    let lp = env.user(&m, u64::MAX / 4);
    for id in 0..8 {
        env.prepare(&m, &keeper, id).unwrap();
        env.send(
            &[Env::ix_initialize_price(&m, &keeper.pubkey(), id, Q64)],
            &[&keeper],
        )
        .unwrap();
        // Shard 5 gets liquidity below the threshold (1000).
        let l = if id == 5 { 999 } else { 1_000_000_000 };
        env.add_liquidity(&m, &lp, id, -1200, 1200, l).unwrap();
    }
    let all: Vec<u8> = (0..8).collect();
    let before = (env.registry(&m), env.market_state(&m));
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &all)],
            &[&keeper],
        ),
        amm_code(AmmError::InitialShardsNotReady),
    );
    assert_eq!(
        (env.registry(&m), env.market_state(&m)),
        before,
        "failed open changes nothing"
    );
    // Topping it up makes the market openable.
    env.add_liquidity(&m, &lp, 5, -1200, 1200, 1_000_000_000)
        .unwrap();
    env.send(
        &[env.ix_open_market(&m, &keeper.pubkey(), &all)],
        &[&keeper],
    )
    .unwrap();
}

#[test]
fn open_market_rejects_wrong_accounts() {
    let mut env = Env::new();
    let m = env.market();
    env.seed_initial_shards(&m, 1_000_000_000);
    let keeper = env.keeper.insecure_clone();
    let all: Vec<u8> = (0..8).collect();
    // Wrong order of shards.
    let mut order = all.clone();
    order.swap(2, 3);
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &order)],
            &[&keeper],
        ),
        amm_code(AmmError::ShardMismatch),
    );
    // Too few shards.
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &all[..7])],
            &[&keeper],
        ),
        amm_code(AmmError::InvalidAccountCount),
    );
    // Vaults of another shard: real balances, but not this shard's.
    let mut i = env.ix_open_market(&m, &keeper.pubkey(), &all);
    let other = shard_addrs(&m, 7, 0);
    i.accounts[3 + 1].pubkey = other.vault_a;
    expect_custom(env.send(&[i], &[&keeper]), amm_code(AmmError::InvalidVault));
    // Shard from another market with the same mints.
    let m2 = env.market_with(m.mint_a, m.mint_b, 2);
    env.seed_initial_shards(&m2, 1_000_000_000);
    let mut i = env.ix_open_market(&m, &keeper.pubkey(), &all);
    let foreign = shard_addrs(&m2, 0, 0);
    i.accounts[3].pubkey = foreign.shard;
    i.accounts[4].pubkey = foreign.vault_a;
    i.accounts[5].pubkey = foreign.vault_b;
    expect_custom(
        env.send(&[i], &[&keeper]),
        amm_code(AmmError::ShardMismatch),
    );
    // Unauthorized signer.
    let stranger = env.user(&m, 0).kp;
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &stranger.pubkey(), &all)],
            &[&stranger],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(
        &[env.ix_open_market(&m, &keeper.pubkey(), &all)],
        &[&keeper],
    )
    .unwrap();
}

#[test]
fn initialize_price_rules() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 0).unwrap();
    let stranger = env.user(&m, 0).kp;
    expect_custom(
        env.send(
            &[Env::ix_initialize_price(&m, &stranger.pubkey(), 0, Q64)],
            &[&stranger],
        ),
        amm_code(AmmError::Unauthorized),
    );
    use sharded_amm::math::tick_math::{MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};
    for bad in [0, MIN_SQRT_PRICE_X64 - 1, MAX_SQRT_PRICE_X64] {
        expect_custom(
            env.send(
                &[Env::ix_initialize_price(&m, &keeper.pubkey(), 0, bad)],
                &[&keeper],
            ),
            amm_code(AmmError::SqrtPriceOutOfRange),
        );
    }
    // Liquidity before a price is rejected.
    let lp = env.user(&m, 1_000_000_000);
    expect_custom(
        env.add_liquidity(&m, &lp, 0, -60, 60, 1_000),
        amm_code(AmmError::PriceNotInitialized),
    );
    env.send(
        &[Env::ix_initialize_price(&m, &keeper.pubkey(), 0, 3 * Q64)],
        &[&keeper],
    )
    .unwrap();
    let s = env.shard_state(&m, 0);
    assert_eq!(s.sqrt_price_x64, 3 * Q64);
    assert_eq!(
        s.tick_current,
        sharded_amm::math::tick_math::tick_at_sqrt_price(3 * Q64).unwrap()
    );
    expect_custom(
        env.send(
            &[Env::ix_initialize_price(&m, &keeper.pubkey(), 0, Q64)],
            &[&keeper],
        ),
        amm_code(AmmError::PriceAlreadyInitialized),
    );
}

#[test]
fn tick_arrays_and_positions_are_bound_to_their_shard_and_owner() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let keeper = env.keeper.insecure_clone();
    let s0 = shard_addrs(&m, 0, 0).shard;
    // Unaligned start.
    expect_custom(
        env.send(
            &[Env::ix_init_tick_array(&m, &keeper.pubkey(), &s0, 60)],
            &[&keeper],
        ),
        amm_code(AmmError::InvalidTickArrayStart),
    );
    // Shard 1's arrays cannot serve shard 0.
    let mut i = env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000);
    let n = i.accounts.len();
    let s1 = shard_addrs(&m, 1, 0).shard;
    i.accounts[n - 2].pubkey = tick_array_pda(&s1, -1920);
    i.accounts[n - 1].pubkey = tick_array_pda(&s1, 0);
    expect_custom(
        env.send(&[i], &[&lp.kp]),
        amm_code(AmmError::TickArrayMismatch),
    );
    // The same array twice.
    let mut i = env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000);
    i.accounts[n - 1].pubkey = i.accounts[n - 2].pubkey;
    expect_custom(
        env.send(&[i], &[&lp.kp]),
        amm_code(AmmError::DuplicateAccount),
    );
    // Missing upper array.
    let mut i = env.ix_increase(&m, &lp, 0, -1200, 1200, 1_000);
    i.accounts.pop();
    expect_custom(
        env.send(&[i], &[&lp.kp]),
        amm_code(AmmError::TickArrayMismatch),
    );
    // Someone else cannot touch the LP's position, even with their own tokens.
    let thief = env.user(&m, 1_000_000_000);
    let mut i = env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000);
    i.accounts[0].pubkey = thief.kp.pubkey();
    i.accounts[6].pubkey = thief.token_a;
    i.accounts[7].pubkey = thief.token_b;
    expect_custom(
        env.send(&[i], &[&thief.kp]),
        anchor_codes::CONSTRAINT_HAS_ONE,
    );
    // Position of shard 0 used with shard 1's accounts.
    let mut i = env.ix_decrease(&m, &lp, 1, -1200, 1200, 1_000);
    i.accounts[3].pubkey = position_pda(&s0, &lp.kp.pubkey(), -1200, 1200);
    expect_custom(env.send(&[i], &[&lp.kp]), anchor_codes::CONSTRAINT_HAS_ONE);
    // Bad ranges.
    for (lo, hi) in [(60, 60), (120, 60), (-1230, 1200), (-443700, 0)] {
        expect_custom(
            env.send(
                &[Env::ix_open_position(&m, &lp.kp.pubkey(), &s0, lo, hi)],
                &[&lp.kp],
            ),
            amm_code(AmmError::InvalidTickRange),
        );
    }
}

#[test]
fn positions_round_trip_and_close() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let lp = env.user(&m, 1_000_000_000_000);
    let (a0, b0) = (env.balance(&lp.token_a), env.balance(&lp.token_b));
    env.add_liquidity(&m, &lp, 3, -600, 1800, 123_456_789)
        .unwrap();
    let (a1, b1) = (env.balance(&lp.token_a), env.balance(&lp.token_b));
    assert!(a1 < a0 && b1 < b0, "in-range position takes both tokens");
    assert_eq!(
        env.shard_state(&m, 3).liquidity,
        1_000_000_000_000 + 123_456_789
    );
    let s3 = shard_addrs(&m, 3, 0).shard;
    let pos = position_pda(&s3, &lp.kp.pubkey(), -600, 1800);
    expect_custom(
        env.send(&[ix_close(&lp, pos)], &[&lp.kp]),
        amm_code(AmmError::PositionNotEmpty),
    );
    // Over-removal fails, exact removal returns at most what was paid.
    expect_custom(
        env.send(
            &[env.ix_decrease(&m, &lp, 3, -600, 1800, 123_456_790)],
            &[&lp.kp],
        ),
        amm_code(AmmError::InsufficientPositionLiquidity),
    );
    env.send(
        &[env.ix_decrease(&m, &lp, 3, -600, 1800, 123_456_789)],
        &[&lp.kp],
    )
    .unwrap();
    let (a2, b2) = (env.balance(&lp.token_a), env.balance(&lp.token_b));
    assert!(
        a2 <= a0 && b2 <= b0 && a0 - a2 <= 1 && b0 - b2 <= 1,
        "rounding costs at most one unit"
    );
    assert_eq!(env.shard_state(&m, 3).liquidity, 1_000_000_000_000);
    // Empty ticks are cleared.
    let arr = env.tick_array(&tick_array_pda(&s3, 0));
    assert!(!arr.tick(1800, 60).unwrap().is_initialized());
    env.send(&[ix_close(&lp, pos)], &[&lp.kp]).unwrap();
    assert!(env.raw(&pos).is_none());
    // Slippage bounds on add.
    env.send(
        &[Env::ix_open_position(&m, &lp.kp.pubkey(), &s3, -600, 1800)],
        &[&lp.kp],
    )
    .unwrap();
    let mut i = env.ix_increase(&m, &lp, 3, -600, 1800, 1_000_000);
    if let Some(d) = i.data.get_mut(8 + 16..8 + 24) {
        d.copy_from_slice(&1u64.to_le_bytes()); // amount_a_max = 1
    }
    expect_custom(
        env.send(&[i], &[&lp.kp]),
        amm_code(AmmError::SlippageExceeded),
    );
}

fn ix_close(lp: &Lp, position: Pubkey) -> solana_instruction::Instruction {
    ix(
        sharded_amm::accounts::ClosePosition {
            owner: lp.kp.pubkey(),
            position,
        },
        sharded_amm::instruction::ClosePosition {},
    )
}

#[test]
fn cancelled_setup_refunds_every_lp() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    let admin = env.admin.insecure_clone();
    let lps: Vec<Lp> = (0..3).map(|_| env.user(&m, 10_000_000_000)).collect();
    let start: Vec<(u64, u64)> = lps
        .iter()
        .map(|l| (env.balance(&l.token_a), env.balance(&l.token_b)))
        .collect();
    for id in 0..3 {
        env.prepare(&m, &keeper, id).unwrap();
        env.send(
            &[Env::ix_initialize_price(&m, &keeper.pubkey(), id, Q64)],
            &[&keeper],
        )
        .unwrap();
        for (k, lp) in lps.iter().enumerate() {
            let w = 60 * (k as i32 + 1);
            env.add_liquidity(&m, lp, id, -w * 10, w * 10, 10_000_000)
                .unwrap();
        }
    }
    expect_custom(
        env.send(&[Env::ix_cancel_setup(&m, &keeper.pubkey())], &[&keeper]),
        amm_code(AmmError::Unauthorized),
    );
    env.send(&[Env::ix_cancel_setup(&m, &admin.pubkey())], &[&admin])
        .unwrap();
    assert_eq!(env.market_state(&m).status, MarketStatus::Cancelled);
    // Nothing new can start.
    expect_custom(
        env.add_liquidity(&m, &lps[0], 0, -60, 60, 1_000),
        amm_code(AmmError::InvalidMarketStatus),
    );
    expect_custom(
        env.prepare(&m, &keeper, 4),
        amm_code(AmmError::InvalidMarketStatus),
    );
    expect_custom(
        env.send(
            &[env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>())],
            &[&keeper],
        ),
        // Shards 3..7 were never prepared, so account loading rejects first.
        3012,
    );
    expect_custom(
        env.send(&[Env::ix_cancel_setup(&m, &admin.pubkey())], &[&admin]),
        amm_code(AmmError::InvalidMarketStatus),
    );
    // Every LP withdraws everything; losses are only rounding units.
    for id in 0..3 {
        for (k, lp) in lps.iter().enumerate() {
            let w = 60 * (k as i32 + 1);
            env.send(
                &[env.ix_decrease(&m, lp, id, -w * 10, w * 10, 10_000_000)],
                &[&lp.kp],
            )
            .unwrap();
            env.send(&[env.ix_collect(&m, lp, id, -w * 10, w * 10)], &[&lp.kp])
                .unwrap();
        }
    }
    for (lp, (a0, b0)) in lps.iter().zip(start) {
        let (a, b) = (env.balance(&lp.token_a), env.balance(&lp.token_b));
        assert!(
            a <= a0 && b <= b0 && a0 - a <= 3 && b0 - b <= 3,
            "refund lost more than rounding"
        );
    }
}

#[test]
fn paused_market_allows_exits_only() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let trader = env.user(&m, 1_000_000_000);
    env.swap(&m, &trader, 0, true, 10_000_000).unwrap();
    let pauser = env.pauser.insecure_clone();
    env.send(
        &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
        &[&pauser],
    )
    .unwrap();
    expect_custom(
        env.swap(&m, &trader, 0, false, 1_000),
        amm_code(AmmError::InvalidMarketStatus),
    );
    expect_custom(
        env.add_liquidity(&m, &lp, 0, -1200, 1200, 1_000),
        amm_code(AmmError::InvalidMarketStatus),
    );
    env.send(&[env.ix_collect(&m, &lp, 0, -1200, 1200)], &[&lp.kp])
        .unwrap();
    env.send(
        &[env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000_000)],
        &[&lp.kp],
    )
    .unwrap();
    let admin = env.admin.insecure_clone();
    env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), false)], &[&admin])
        .unwrap();
    env.swap(&m, &trader, 0, false, 1_000).unwrap();
    let _ = Keypair::new();
}
