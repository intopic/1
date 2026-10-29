//! D-04, D-05, D-07 (mainnet analysis): one `ProtocolConfig` holds every
//! role. Markets copy the roles, never take them from their creator, and a
//! rotation (timelocked, two-step for the admin) or an emergency pauser
//! removal reaches every market. Mints with a freeze authority are refused
//! unless allowlisted. LP exits never depend on any of it.

mod common;

use common::*;
use sharded_amm::constants::{FEE_TIMELOCK_SECONDS, RISK_MINT_AUTHORITY_A, RISK_MINT_AUTHORITY_B};
use sharded_amm::errors::AmmError;
use sharded_amm::instruction as ixs;
use sharded_amm::state::{MarketStatus, ProtocolSettings, Roles};
use solana_keypair::Keypair;
use solana_signer::Signer;

fn funded(env: &mut Env) -> Keypair {
    let kp = Keypair::new();
    env.svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
    kp
}

fn new_roles(env: &mut Env) -> (Roles, [Keypair; 4]) {
    let ks = [funded(env), funded(env), funded(env), funded(env)];
    let roles = Roles {
        admin: ks[0].pubkey(),
        keeper: ks[1].pubkey(),
        pauser: ks[2].pubkey(),
        revenue_authority: ks[3].pubkey(),
    };
    (roles, ks)
}

#[test]
fn only_the_upgrade_authority_initializes_the_protocol_once() {
    // A fresh program with no protocol account yet.
    let mut env = Env::new();
    let admin = env.admin.insecure_clone();
    let protocol = env.protocol();
    assert_eq!(protocol.roles, env.roles());
    assert_eq!(protocol.roles_version, 0);
    // A second initialization fails (the PDA exists).
    let ix = env.ix_initialize_protocol(&admin.pubkey(), env.roles(), default_settings());
    expect_err(env.send(&[ix], &[&admin]));

    // On a program whose protocol account does not exist yet, a stranger
    // cannot take the roles, nor can anyone if there is no upgrade authority.
    let mut fresh = Env::new();
    let pda = protocol_pda();
    fresh.svm.set_account(pda, Default::default()).unwrap();
    let stranger = funded(&mut fresh);
    let ix = fresh.ix_initialize_protocol(&stranger.pubkey(), fresh.roles(), default_settings());
    expect_custom(
        fresh.send(&[ix], &[&stranger]),
        amm_code(AmmError::NotUpgradeAuthority),
    );
    fresh.set_upgrade_authority(None);
    let admin = fresh.admin.insecure_clone();
    let ix = fresh.ix_initialize_protocol(&admin.pubkey(), fresh.roles(), default_settings());
    expect_custom(
        fresh.send(&[ix], &[&admin]),
        amm_code(AmmError::NotUpgradeAuthority),
    );
    fresh.set_upgrade_authority(Some(&admin.pubkey()));
    let ix = fresh.ix_initialize_protocol(&admin.pubkey(), fresh.roles(), default_settings());
    fresh.send(&[ix], &[&admin]).expect("upgrade authority");
}

#[test]
fn any_creator_gets_a_market_run_by_the_protocol() {
    let mut env = Env::new();
    let creator = funded(&mut env);
    let (a, b) = env.sorted_mints();
    let ix = env.ix_initialize_market(&creator.pubkey(), a, b, env.init_args(7, default_params()));
    env.send(&[ix], &[&creator]).expect("anyone may create");
    let m = market_at(a, b, 7);
    let cfg = env.market_state(&m);
    assert_eq!(cfg.creator, creator.pubkey());
    assert_eq!(
        (cfg.admin, cfg.keeper, cfg.pauser, cfg.revenue_authority),
        (
            env.admin.pubkey(),
            env.keeper.pubkey(),
            env.pauser.pubkey(),
            env.revenue.pubkey()
        )
    );
    assert_eq!(cfg.rebalancers[0], env.keeper.pubkey());
    // Test mints keep a mint authority: flagged, not refused.
    assert_eq!(
        cfg.risk_flags,
        RISK_MINT_AUTHORITY_A | RISK_MINT_AUTHORITY_B
    );
    // The creator holds no role.
    for i in [
        Env::ix_update_fees(&m, &creator.pubkey(), 500, 1_600),
        Env::ix_cancel_setup(&m, &creator.pubkey()),
    ] {
        expect_custom(
            env.send(&[i], &[&creator]),
            amm_code(AmmError::Unauthorized),
        );
    }
    assert!(env.prepare(&m, &creator, 0).is_err());
}

fn market_at(mint_a: Pubkey, mint_b: Pubkey, id: u64) -> Market {
    let market = market_pda(&mint_a, &mint_b, id);
    Market {
        mint_a,
        mint_b,
        market_id: id,
        market,
        registry: pda(&[sharded_amm::constants::REGISTRY_SEED, market.as_ref()]),
    }
}

#[test]
fn role_rotation_is_timelocked_reaches_markets_and_needs_admin_acceptance() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let admin = env.admin.insecure_clone();
    let old_pauser = env.pauser.insecure_clone();
    let (roles, [new_admin, new_keeper, new_pauser, _]) = new_roles(&mut env);
    let stranger = funded(&mut env);

    let propose = Env::ix_protocol(&admin.pubkey(), ixs::ProposeRoles { roles });
    expect_custom(
        env.send(
            &[Env::ix_protocol(
                &stranger.pubkey(),
                ixs::ProposeRoles { roles },
            )],
            &[&stranger],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(&[propose], &[&admin]).expect("propose_roles");
    let apply = || Env::ix_protocol(&admin.pubkey(), ixs::ApplyRoles {});
    env.warp(FEE_TIMELOCK_SECONDS - 10);
    expect_custom(
        env.send(&[apply()], &[&admin]),
        amm_code(AmmError::TimelockNotElapsed),
    );
    env.warp(10);
    env.send(&[apply()], &[&admin]).expect("apply_roles");
    let p = env.protocol();
    assert_eq!(p.roles.admin, admin.pubkey(), "admin waits for acceptance");
    assert_eq!(p.pending_admin, new_admin.pubkey());
    assert_eq!(
        (p.roles.keeper, p.roles.pauser),
        (roles.keeper, roles.pauser)
    );

    // The market's copies are stale: every privileged instruction fails
    // until someone syncs, so the old keys lose power everywhere at once.
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &old_pauser.pubkey(), true)],
            &[&old_pauser],
        ),
        amm_code(AmmError::StaleRoles),
    );
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &new_pauser.pubkey(), true)],
            &[&new_pauser],
        ),
        amm_code(AmmError::StaleRoles),
    );
    // LP exits and swaps never read the protocol account.
    let swapper = env.user(&m, 1_000_000_000);
    env.swap(&m, &swapper, 0, true, 1_000_000)
        .expect("swap with stale roles");
    let i = env.ix_decrease(&m, &lp, 1, -1200, 1200, 1_000);
    env.send(&[i], &[&lp.kp]).expect("exit with stale roles");

    // Anyone can sync; it copies the protocol's roles and nothing else.
    env.send(&[Env::ix_sync_roles(&m)], &[&stranger])
        .expect("sync");
    expect_custom(
        env.send(&[Env::ix_sync_roles(&m)], &[&stranger]),
        amm_code(AmmError::NoPendingChange),
    );
    let cfg = env.market_state(&m);
    assert_eq!(cfg.roles_version, env.protocol().roles_version);
    assert_eq!((cfg.keeper, cfg.pauser), (roles.keeper, roles.pauser));
    assert_eq!(
        cfg.rebalancers[0],
        new_keeper.pubkey(),
        "keeper's slot moves"
    );
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &old_pauser.pubkey(), true)],
            &[&old_pauser],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(
        &[Env::ix_set_paused(&m, &new_pauser.pubkey(), true)],
        &[&new_pauser],
    )
    .expect("new pauser");
    assert_eq!(env.market_state(&m).status, MarketStatus::Paused);

    // Second step: only the pending admin can accept.
    expect_custom(
        env.send(
            &[Env::ix_protocol(&stranger.pubkey(), ixs::AcceptAdmin {})],
            &[&stranger],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(
        &[Env::ix_protocol(&new_admin.pubkey(), ixs::AcceptAdmin {})],
        &[&new_admin],
    )
    .expect("accept_admin");
    env.send(&[Env::ix_sync_roles(&m)], &[&stranger])
        .expect("sync");
    expect_custom(
        env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), false)], &[&admin]),
        amm_code(AmmError::Unauthorized),
    );
    env.send(
        &[Env::ix_set_paused(&m, &new_admin.pubkey(), false)],
        &[&new_admin],
    )
    .expect("new admin unpauses");
    expect_custom(
        env.send(
            &[Env::ix_protocol(
                &admin.pubkey(),
                ixs::ProposeRoles { roles },
            )],
            &[&admin],
        ),
        amm_code(AmmError::Unauthorized),
    );
}

#[test]
fn admin_removes_a_leaked_pauser_immediately() {
    let mut env = Env::new();
    let (m, _lp) = env.open_seeded_market();
    let admin = env.admin.insecure_clone();
    let pauser = env.pauser.insecure_clone();
    expect_custom(
        env.send(
            &[Env::ix_protocol(&pauser.pubkey(), ixs::RemovePauser {})],
            &[&pauser],
        ),
        amm_code(AmmError::Unauthorized),
    );
    env.send(
        &[Env::ix_protocol(&admin.pubkey(), ixs::RemovePauser {})],
        &[&admin],
    )
    .expect("no timelock");
    assert_eq!(env.protocol().roles.pauser, Pubkey::default());
    env.send(&[Env::ix_sync_roles(&m)], &[&admin])
        .expect("sync");
    expect_custom(
        env.send(
            &[Env::ix_set_paused(&m, &pauser.pubkey(), true)],
            &[&pauser],
        ),
        amm_code(AmmError::Unauthorized),
    );
    // The admin can still pause.
    env.send(&[Env::ix_set_paused(&m, &admin.pubkey(), true)], &[&admin])
        .expect("admin pauses");
}

#[test]
fn settings_are_timelocked_and_freeze_authority_is_refused_unless_listed() {
    let mut env = Env::new();
    let admin = env.admin.insecure_clone();
    let freezer = Pubkey::new_unique();
    let usdc_like = env.create_mint_with(6, Some(&freezer), Some(&freezer));
    let meme = env.create_mint_with(9, None, Some(&freezer));
    let clean = env.create_mint_with(9, None, None);
    let sorted = |x: Pubkey, y: Pubkey| {
        if x.to_bytes() < y.to_bytes() {
            (x, y)
        } else {
            (y, x)
        }
    };

    let create = |env: &mut Env, x: Pubkey, y: Pubkey, id: u64| {
        let (a, b) = sorted(x, y);
        let ix =
            env.ix_initialize_market(&admin.pubkey(), a, b, env.init_args(id, default_params()));
        env.send(&[ix], &[&admin])
    };
    for (x, y) in [(meme, clean), (usdc_like, clean)] {
        expect_custom(
            create(&mut env, x, y, 1),
            amm_code(AmmError::FreezeAuthorityNotAllowed),
        );
    }

    // List the stablecoin through the timelock.
    let mut s: ProtocolSettings = env.protocol().settings;
    s.freeze_allowlist[0] = usdc_like;
    env.send(
        &[Env::ix_protocol(
            &admin.pubkey(),
            ixs::ProposeSettings { settings: s },
        )],
        &[&admin],
    )
    .expect("propose_settings");
    let apply = Env::ix_protocol(&admin.pubkey(), ixs::ApplySettings {});
    expect_custom(
        env.send(std::slice::from_ref(&apply), &[&admin]),
        amm_code(AmmError::TimelockNotElapsed),
    );
    env.warp(FEE_TIMELOCK_SECONDS);
    env.send(&[apply], &[&admin]).expect("apply_settings");
    assert_eq!(env.protocol().settings, s);

    create(&mut env, usdc_like, clean, 2).expect("allowlisted stablecoin");
    let (a, b) = sorted(usdc_like, clean);
    let cfg = env.market_state(&market_at(a, b, 2));
    let flag = if a == usdc_like {
        RISK_MINT_AUTHORITY_A
    } else {
        RISK_MINT_AUTHORITY_B
    };
    assert_eq!(
        cfg.risk_flags, flag,
        "only the stablecoin has a mint authority"
    );
    expect_custom(
        create(&mut env, meme, clean, 3),
        amm_code(AmmError::FreezeAuthorityNotAllowed),
    );
}

#[test]
fn launch_profiles_create_markets_with_their_fees() {
    use sharded_amm::state::{PROFILE_MAJOR, PROFILE_MEMECOIN, PROFILE_STABLE};
    let mut env = Env::new();
    env.force_settings(ProtocolSettings::launch());
    let admin = env.admin.insecure_clone();
    let (a, b) = env.sorted_mints();
    for (id, profile, spacing, fee, max) in [
        (1u64, PROFILE_STABLE, 1u16, 100u32, 100u32),
        (2, PROFILE_MAJOR, 10, 500, 3_000),
        (3, PROFILE_MEMECOIN, 200, 10_000, 20_000),
    ] {
        let mut args = env.init_args(id, default_params_for(&env, profile));
        args.profile = profile;
        let ix = env.ix_initialize_market(&admin.pubkey(), a, b, args);
        env.send(&[ix], &[&admin]).expect("launch profile");
        let cfg = env.market_state(&market_at(a, b, id));
        assert_eq!(
            (
                cfg.params.tick_spacing,
                cfg.params.fee_rate_ppm,
                cfg.params.protocol_share_bps
            ),
            (spacing, fee, 1_600)
        );
        assert_eq!(cfg.dynamic_fees.max_fee_rate_ppm, max);
    }
}

fn default_params_for(env: &Env, profile: u8) -> sharded_amm::state::MarketParams {
    env.protocol().settings.profiles[usize::from(profile)].params()
}
