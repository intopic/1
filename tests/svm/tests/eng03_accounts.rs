//! ENG-03: market initialization, shard preparation, PDA/vault custody and
//! negative account-substitution tests, executed in LiteSVM against the
//! SBF build of sharded-amm.

mod common;

use common::*;
use sharded_amm::errors::AmmError;
use sharded_amm::state::{MarketParams, MarketStatus, ShardStatus};
use solana_keypair::Keypair;
use solana_signer::Signer;

#[test]
fn initialize_market_creates_config_and_empty_registry() {
    let mut env = Env::new();
    let m = env.market();
    let cfg = env.market_state(&m);
    assert_eq!(cfg.mint_a, m.mint_a);
    assert_eq!(cfg.mint_b, m.mint_b);
    assert_eq!(cfg.token_program, TOKEN_PROGRAM);
    assert_eq!(cfg.admin, env.admin.pubkey());
    assert_eq!(cfg.keeper, env.keeper.pubkey());
    assert_eq!(cfg.revenue_authority, env.revenue.pubkey());
    assert_eq!(cfg.pauser, env.pauser.pubkey());
    assert_eq!(cfg.params, default_params());
    assert_eq!(cfg.status, MarketStatus::Initializing);
    let reg = env.registry(&m);
    assert_eq!(reg.market, m.market);
    assert_eq!(
        (reg.prepared_mask, reg.active_mask, reg.registry_version),
        (0, 0, 0)
    );
}

#[test]
fn initialize_market_rejects_non_canonical_or_equal_mints() {
    let mut env = Env::new();
    let (a, b) = env.sorted_mints();
    let admin = env.admin.insecure_clone();
    let ix = env.ix_initialize_market(&admin.pubkey(), b, a, env.init_args(1, default_params()));
    expect_custom(
        env.send(&[ix], &[&admin]),
        amm_code(AmmError::MintsNotCanonical),
    );
    let ix = env.ix_initialize_market(&admin.pubkey(), a, a, env.init_args(1, default_params()));
    expect_custom(
        env.send(&[ix], &[&admin]),
        amm_code(AmmError::MintsNotCanonical),
    );
}

#[test]
fn initialize_market_rejects_token_2022_mint_and_wrong_token_program() {
    let mut env = Env::new();
    let a = env.create_mint(9, TOKEN_2022_PROGRAM);
    let b = env.create_mint(6, TOKEN_PROGRAM);
    let (a, b) = if a.to_bytes() < b.to_bytes() {
        (a, b)
    } else {
        (b, a)
    };
    let admin = env.admin.insecure_clone();
    let ix = env.ix_initialize_market(&admin.pubkey(), a, b, env.init_args(1, default_params()));
    expect_custom(
        env.send(&[ix], &[&admin]),
        anchor_codes::ACCOUNT_OWNED_BY_WRONG_PROGRAM,
    );

    let (a, b) = env.sorted_mints();
    let mut ix =
        env.ix_initialize_market(&admin.pubkey(), a, b, env.init_args(1, default_params()));
    let tp = ix
        .accounts
        .iter_mut()
        .find(|m| m.pubkey == TOKEN_PROGRAM)
        .unwrap();
    tp.pubkey = TOKEN_2022_PROGRAM;
    expect_custom(env.send(&[ix], &[&admin]), anchor_codes::INVALID_PROGRAM_ID);
}

#[test]
fn initialize_market_rejects_out_of_bounds_params() {
    let mut env = Env::new();
    let (a, b) = env.sorted_mints();
    let admin = env.admin.insecure_clone();
    let p = default_params();
    for (params, err) in [
        (
            MarketParams {
                fee_rate_ppm: 100_001,
                ..p
            },
            AmmError::InvalidFeeRate,
        ),
        (
            MarketParams {
                protocol_share_bps: 5_001,
                ..p
            },
            AmmError::InvalidProtocolShare,
        ),
        (
            MarketParams {
                tick_spacing: 0,
                ..p
            },
            AmmError::InvalidTickSpacing,
        ),
        (
            MarketParams {
                tick_spacing: 1_001,
                ..p
            },
            AmmError::InvalidTickSpacing,
        ),
    ] {
        let ix = env.ix_initialize_market(&admin.pubkey(), a, b, env.init_args(1, params));
        expect_custom(env.send(&[ix], &[&admin]), amm_code(err));
    }
    // Candidate fee tiers and spacings from ADR-0004 are accepted.
    for (id, fee, spacing) in [(10u64, 500u32, 10u16), (11, 1_000, 60), (12, 3_000, 10)] {
        let params = MarketParams {
            fee_rate_ppm: fee,
            protocol_share_bps: 2_000,
            tick_spacing: spacing,
        };
        let ix = env.ix_initialize_market(&admin.pubkey(), a, b, env.init_args(id, params));
        env.send(&[ix], &[&admin]).expect("valid params");
    }
}

#[test]
fn initialize_market_twice_fails() {
    let mut env = Env::new();
    let m = env.market();
    let admin = env.admin.insecure_clone();
    let ix = env.ix_initialize_market(
        &admin.pubkey(),
        m.mint_a,
        m.mint_b,
        env.init_args(1, default_params()),
    );
    expect_err(env.send(&[ix], &[&admin]));
}

#[test]
fn prepare_eight_initial_shards_with_isolated_vaults() {
    let mut env = Env::new();
    let m = env.market();
    let admin = env.admin.insecure_clone();
    let keeper = env.keeper.insecure_clone();
    let mut vaults = std::collections::BTreeSet::new();
    let mut cus = Vec::new();
    for id in 0..8u8 {
        // Admin may prepare during setup; keeper always may.
        let signer = if id % 2 == 0 { &admin } else { &keeper };
        cus.push(env.prepare(&m, signer, id).expect("prepare"));
        let a = shard_addrs(&m, id, 0);
        let s = env.shard_state(&m, id);
        assert_eq!(
            (s.market, s.id, s.generation, s.status),
            (m.market, id, 0, ShardStatus::Prepared)
        );
        assert_eq!((s.vault_a, s.vault_b), (a.vault_a, a.vault_b));
        assert_eq!(
            (
                s.sqrt_price_x64,
                s.liquidity,
                s.protocol_fees_a,
                s.protocol_fees_b
            ),
            (0, 0, 0, 0)
        );
        for (vault, mint) in [(a.vault_a, m.mint_a), (a.vault_b, m.mint_b)] {
            let acc = env.raw(&vault).unwrap();
            assert_eq!(acc.owner, TOKEN_PROGRAM);
            let (vmint, vowner, amount) = token_account_fields(&acc.data);
            assert_eq!((vmint, vowner, amount), (mint, a.vault_authority, 0));
            assert!(vaults.insert(vault), "vault shared between shards");
        }
        // The vault authority is a data-less PDA that holds nothing.
        assert!(env.raw(&a.vault_authority).is_none());
    }
    assert_eq!(vaults.len(), 16);
    let reg = env.registry(&m);
    assert_eq!(
        (reg.prepared_mask, reg.active_mask, reg.registry_version),
        (0xff, 0, 8)
    );
    assert_eq!(env.market_state(&m).status, MarketStatus::Initializing);
    println!("prepare_shard CU: {cus:?}");
}

#[test]
fn prepare_requires_role() {
    let mut env = Env::new();
    let m = env.market();
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 10_000_000_000).unwrap();
    for who in [
        &stranger,
        &env.revenue.insecure_clone(),
        &env.pauser.insecure_clone(),
    ] {
        expect_custom(env.prepare(&m, who, 0), amm_code(AmmError::Unauthorized));
    }
    assert_eq!(env.registry(&m).prepared_mask, 0);
}

#[test]
fn prepare_authority_must_sign() {
    let mut env = Env::new();
    let m = env.market();
    let admin = env.admin.insecure_clone();
    let mut accounts = env.prepare_accounts(&m, &env.keeper.pubkey(), 0, 0);
    accounts.payer = admin.pubkey();
    let mut ix = Env::ix_prepare(accounts, 0, 0);
    let keeper = env.keeper.pubkey();
    ix.accounts
        .iter_mut()
        .find(|a| a.pubkey == keeper)
        .unwrap()
        .is_signer = false;
    expect_custom(env.send(&[ix], &[&admin]), anchor_codes::ACCOUNT_NOT_SIGNER);
}

#[test]
fn prepare_rejects_bad_ids_and_generations() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    expect_custom(
        env.prepare(&m, &keeper, 8),
        amm_code(AmmError::NotAnInitialShard),
    );
    expect_custom(
        env.prepare(&m, &keeper, 55),
        amm_code(AmmError::NotAnInitialShard),
    );
    for id in [56u8, 63, 255] {
        expect_custom(
            env.prepare(&m, &keeper, id),
            amm_code(AmmError::ShardIdOutOfRange),
        );
    }
    let ix = Env::ix_prepare(env.prepare_accounts(&m, &keeper.pubkey(), 0, 1), 0, 1);
    expect_custom(
        env.send(&[ix], &[&keeper]),
        amm_code(AmmError::UnsupportedGeneration),
    );
    assert_eq!(env.registry(&m).registry_version, 0);
}

#[test]
fn prepare_same_shard_twice_fails_without_state_change() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 3).unwrap();
    let before = env.raw(&m.registry).unwrap().data;
    expect_err(env.prepare(&m, &keeper, 3));
    assert_eq!(env.raw(&m.registry).unwrap().data, before);
}

#[test]
fn prepare_rejects_substituted_accounts() {
    let mut env = Env::new();
    let m = env.market();
    let other = env.market(); // second market with different mints
    let keeper = env.keeper.insecure_clone();
    let id = 0u8;

    // Mint not belonging to the market.
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.mint_b = other.mint_b;
    expect_err(env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]));

    // Registry of another market.
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.registry = other.registry;
    expect_custom(
        env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]),
        anchor_codes::CONSTRAINT_SEEDS,
    );

    // Wrong vault authority PDA (another shard's).
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.vault_authority = shard_addrs(&m, 1, 0).vault_authority;
    expect_custom(
        env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]),
        anchor_codes::CONSTRAINT_SEEDS,
    );

    // Vault alias: same address for A and B.
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.vault_b = acc.vault_a;
    expect_err(env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]));

    // Shard PDA of a different ID than the argument.
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.shard = shard_addrs(&m, 1, 0).shard;
    expect_custom(
        env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]),
        anchor_codes::CONSTRAINT_SEEDS,
    );

    // Token-2022 program in place of SPL Token.
    let mut acc = env.prepare_accounts(&m, &keeper.pubkey(), id, 0);
    acc.token_program = TOKEN_2022_PROGRAM;
    expect_err(env.send(&[Env::ix_prepare(acc, id, 0)], &[&keeper]));

    // None of the failures changed the registry.
    assert_eq!(env.registry(&m).prepared_mask, 0);
    assert_eq!(env.registry(&m).registry_version, 0);
    // And the correct accounts still work.
    env.prepare(&m, &keeper, id).unwrap();
}

#[test]
fn preparing_one_shard_leaves_others_byte_identical() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 0).unwrap();
    let a0 = shard_addrs(&m, 0, 0);
    let snapshot: Vec<_> = [a0.shard, a0.vault_a, a0.vault_b]
        .iter()
        .map(|k| env.raw(k).unwrap())
        .collect();
    for id in 1..8 {
        env.prepare(&m, &keeper, id).unwrap();
    }
    let after: Vec<_> = [a0.shard, a0.vault_a, a0.vault_b]
        .iter()
        .map(|k| env.raw(k).unwrap())
        .collect();
    assert_eq!(snapshot, after);
}

/// Account sizes are ABI (docs/account-layout.md); a change needs a schema bump.
#[test]
fn account_sizes_match_layout_doc() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    env.prepare(&m, &keeper, 0).unwrap();
    assert_eq!(env.raw(&m.market).unwrap().data.len(), 360);
    assert_eq!(env.raw(&m.registry).unwrap().data.len(), 65);
    assert_eq!(
        env.raw(&shard_addrs(&m, 0, 0).shard).unwrap().data.len(),
        296
    );
    env.send(
        &[Env::ix_initialize_price(&m, &keeper.pubkey(), 0, Q64)],
        &[&keeper],
    )
    .unwrap();
    let lp = env.user(&m, 1_000_000_000);
    env.add_liquidity(&m, &lp, 0, -60, 60, 1_000).unwrap();
    let shard = shard_addrs(&m, 0, 0).shard;
    assert_eq!(
        env.raw(&tick_array_pda(&shard, -1920)).unwrap().data.len(),
        3128
    );
    assert_eq!(
        env.raw(&position_pda(&shard, &lp.kp.pubkey(), -60, 60))
            .unwrap()
            .data
            .len(),
        177
    );
}
