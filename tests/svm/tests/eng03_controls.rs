//! ENG-03 part 2: pause/resume roles and bounded fee updates.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::state::{MarketParams, MarketStatus};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn open_market(env: &mut Env) -> Market {
    let m = env.market();
    env.force_market_status(&m, MarketStatus::Open);
    m
}

#[test]
fn pauser_pauses_only_admin_resumes() {
    let mut env = Env::new();
    let m = open_market(&mut env);
    let (admin, keeper, revenue, pauser) = (
        env.admin.insecure_clone(),
        env.keeper.insecure_clone(),
        env.revenue.insecure_clone(),
        env.pauser.insecure_clone(),
    );
    for who in [&keeper, &revenue] {
        let ix = Env::ix_set_paused(&m, &who.pubkey(), true);
        expect_custom(env.send(&[ix], &[who]), amm_code(AmmError::Unauthorized));
    }
    env.send(
        &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
        &[&pauser],
    )
    .unwrap();
    assert_eq!(env.market_state(&m).status, MarketStatus::Paused);
    // Pausing twice is a status error, not a silent no-op.
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
            &[&pauser],
        ),
        amm_code(AmmError::InvalidMarketStatus),
    );
    // A compromised pauser can halt but not resume.
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &pauser.pubkey(), false)],
            &[&pauser],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), false)], &[&admin])
        .unwrap();
    assert_eq!(env.market_state(&m).status, MarketStatus::Open);
}

#[test]
fn paused_market_blocks_prepare() {
    let mut env = Env::new();
    let m = open_market(&mut env);
    let pauser = env.pauser.insecure_clone();
    let keeper = env.keeper.insecure_clone();
    env.send(
        &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
        &[&pauser],
    )
    .unwrap();
    expect_custom(
        env.prepare(&m, &keeper, 8),
        amm_code(AmmError::InvalidMarketStatus),
    );
}

#[test]
fn cannot_pause_while_initializing() {
    let mut env = Env::new();
    let m = env.market();
    let pauser = env.pauser.insecure_clone();
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
            &[&pauser],
        ),
        amm_code(AmmError::InvalidMarketStatus),
    );
}

#[test]
fn fee_update_is_timelocked_admin_only_bounded_spacing_unchanged() {
    let mut env = Env::new();
    let m = env.market();
    let admin = env.admin.insecure_clone();
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();
    for who in [
        &stranger,
        &env.keeper.insecure_clone(),
        &env.pauser.insecure_clone(),
    ] {
        let ix = Env::ix_update_fees(&m, &who.pubkey(), 500, 2_000);
        expect_custom(env.send(&[ix], &[who]), amm_code(AmmError::Unauthorized));
        let ix = Env::ix_apply_fees(&m, &who.pubkey());
        expect_custom(env.send(&[ix], &[who]), amm_code(AmmError::Unauthorized));
    }
    expect_custom(
        env.send(
            &[Env::ix_update_fees(&m, &admin.pubkey(), 100_001, 2_000)],
            &[&admin],
        ),
        amm_code(AmmError::InvalidFeeRate),
    );
    expect_custom(
        env.send(
            &[Env::ix_update_fees(&m, &admin.pubkey(), 500, 5_001)],
            &[&admin],
        ),
        amm_code(AmmError::InvalidProtocolShare),
    );
    expect_custom(
        env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin]),
        amm_code(AmmError::NoPendingChange),
    );
    // Candidate tiers from ADR-0004, each behind the delay.
    for fee in [500u32, 1_000, 3_000] {
        env.send(
            &[Env::ix_update_fees(&m, &admin.pubkey(), fee, 2_000)],
            &[&admin],
        )
        .unwrap();
        let before = env.market_state(&m).params;
        expect_custom(
            env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin]),
            amm_code(AmmError::TimelockNotElapsed),
        );
        env.warp(sharded_amm::constants::FEE_TIMELOCK_SECONDS - 10);
        expect_custom(
            env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin]),
            amm_code(AmmError::TimelockNotElapsed),
        );
        assert_eq!(
            env.market_state(&m).params,
            before,
            "nothing changes during the delay"
        );
        env.warp(10);
        env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin])
            .unwrap();
        assert_eq!(
            env.market_state(&m).params,
            MarketParams {
                fee_rate_ppm: fee,
                protocol_share_bps: 2_000,
                tick_spacing: 60
            }
        );
    }
    // A cancelled proposal cannot be applied later.
    env.send(&[Env::ix_update_fees(&m, &admin.pubkey(), 0, 0)], &[&admin])
        .unwrap();
    env.send(&[Env::ix_cancel_fee_change(&m, &admin.pubkey())], &[&admin])
        .unwrap();
    env.warp(sharded_amm::constants::FEE_TIMELOCK_SECONDS * 2);
    expect_custom(
        env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin]),
        amm_code(AmmError::NoPendingChange),
    );
    assert_eq!(env.market_state(&m).params.fee_rate_ppm, 3_000);
}

#[test]
fn control_instructions_reject_foreign_market_account() {
    let mut env = Env::new();
    let m = env.market();
    let admin = env.admin.insecure_clone();
    // A token vault (owned by SPL Token) passed as the market.
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 0).unwrap();
    let fake = Market {
        market: shard_addrs(&m, 0, 0).vault_a,
        ..m
    };
    expect_custom(
        env.send(
            &[Env::ix_update_fees(&fake, &admin.pubkey(), 500, 2_000)],
            &[&admin],
        ),
        anchor_codes::ACCOUNT_OWNED_BY_WRONG_PROGRAM,
    );
}
