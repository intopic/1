//! ADR-0006: `rebalance_pair` and the pause flags on the real SBF program.
//!
//! - Capital-free: a caller with no tokens closes a gap between two shards
//!   and receives exactly the host engine's predicted profit; both shards
//!   end exactly where the host predicts and the other six are untouched.
//! - Equivalent to two plain swaps with the same amounts and limits, run by
//!   a funded trader in an identical market.
//! - Only listed rebalancers (keeper at creation, changed by the admin
//!   behind the timelock) may call it.
//! - Refused: same shard twice, no price gap, no profit, wrong profit
//!   account, paused.
//! - Pause flags: roles, swaps/deposits/rebalance bits; LP exits never
//!   blocked. The same flags on one shard stop only that shard; with only
//!   the swap bit set the listed rebalancers can bring its price back, and
//!   `reopen_shard` lifts it only within the deviation from the median of
//!   three or more open shards.

mod common;

use common::*;
use sharded_amm::constants::{PAUSE_DEPOSITS, PAUSE_REBALANCE, PAUSE_SWAPS};
use sharded_amm::engine::{rebalance, Pool, RebalanceParams};
use sharded_amm::errors::AmmError;
use sharded_amm::instructions::RebalanceArgs;
use sharded_amm::state::TickArray;
use solana_signer::Signer;

/// Opens the seeded market and pushes shard 0's price down with a sale of A,
/// so A is cheaper on shard 0 than on the other seven.
fn diverged() -> (Env, Market, Lp) {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000_000);
    env.swap(&m, &t, 0, true, 20_000_000_000).unwrap();
    assert!(env.shard_state(&m, 0).tick_current < -100);
    (env, m, lp)
}

/// `lo` buys A (B -> A), `hi` sells it (A -> B).
fn arrays(env: &Env, m: &Market, x: u8, y: u8) -> (Vec<Pubkey>, Vec<Pubkey>) {
    let px = env.shard_state(m, x).sqrt_price_x64;
    let py = env.shard_state(m, y).sqrt_price_x64;
    let x_is_lo = px < py;
    (
        env.swap_arrays(m, x, !x_is_lo),
        env.swap_arrays(m, y, x_is_lo),
    )
}

#[allow(clippy::too_many_arguments)]
fn ix_rebalance(
    m: &Market,
    caller: &Pubkey,
    x: u8,
    y: u8,
    caller_token: Pubkey,
    args: RebalanceArgs,
    ax: &[Pubkey],
    ay: &[Pubkey],
) -> solana_instruction::Instruction {
    let (sx, sy) = (shard_addrs(m, x, 0), shard_addrs(m, y, 0));
    let i = ix(
        sharded_amm::accounts::RebalancePair {
            caller: *caller,
            market: m.market,
            registry: m.registry,
            shard_x: sx.shard,
            vault_x_a: sx.vault_a,
            vault_x_b: sx.vault_b,
            vault_authority_x: sx.vault_authority,
            shard_y: sy.shard,
            vault_y_a: sy.vault_a,
            vault_y_b: sy.vault_b,
            vault_authority_y: sy.vault_authority,
            caller_token,
            token_program: TOKEN_PROGRAM,
        },
        sharded_amm::instruction::RebalancePair { args },
    );
    with_remaining(i, &[ax, ay].concat())
}

fn args(input_is_a: bool, amount_in: u64, arrays_x: usize) -> RebalanceArgs {
    RebalanceArgs {
        input_is_a,
        amount_in,
        min_profit: 0,
        arrays_x: arrays_x as u8,
        deadline: i64::MAX,
    }
}

/// Host prediction on copies of the on-chain accounts: (pools after, profit).
fn predict(
    env: &Env,
    m: &Market,
    lo: u8,
    hi: u8,
    input_is_a: bool,
    amount_in: u64,
) -> Result<(Pool, Pool, u64), AmmError> {
    let p = env.market_state(m).params;
    let (mut pl, mut ph) = (
        Pool::load(&env.shard_state(m, lo)).unwrap(),
        Pool::load(&env.shard_state(m, hi)).unwrap(),
    );
    let mut cl: Vec<TickArray> = env
        .swap_arrays(m, lo, false)
        .iter()
        .map(|k| env.tick_array(k))
        .collect();
    let mut ch: Vec<TickArray> = env
        .swap_arrays(m, hi, true)
        .iter()
        .map(|k| env.tick_array(k))
        .collect();
    let mut rl: Vec<&mut TickArray> = cl.iter_mut().collect();
    let mut rh: Vec<&mut TickArray> = ch.iter_mut().collect();
    let out = rebalance(
        &mut pl,
        &mut rl,
        &mut ph,
        &mut rh,
        RebalanceParams {
            input_is_a,
            amount_in,
            min_profit: 0,
            fee_lo_ppm: p.fee_rate_ppm,
            fee_hi_ppm: p.fee_rate_ppm,
            protocol_share_bps: p.protocol_share_bps,
            tick_spacing: p.tick_spacing,
        },
    )?;
    Ok((pl, ph, out.profit))
}

fn lane_bytes(env: &Env, m: &Market, id: u8) -> Vec<u8> {
    let a = shard_addrs(m, id, 0);
    [a.shard, a.vault_a, a.vault_b]
        .iter()
        .flat_map(|k| env.raw(k).unwrap().data)
        .collect()
}

#[test]
fn capital_free_and_exactly_as_predicted() {
    for (input_is_a, x, y) in [(false, 0u8, 1u8), (true, 1, 0), (false, 2, 0), (true, 0, 3)] {
        let (mut env, m, _) = diverged();
        // Only shard 0 diverged; pick the pair and the direction.
        let (lo, hi) = if x == 0 { (x, y) } else { (y, x) };
        let amount = 2_000_000_000;
        let (pl, ph, profit) = predict(&env, &m, lo, hi, input_is_a, amount).unwrap();
        assert!(profit > 0);
        let caller = env.user(&m, 0);
        env.set_rebalancers(&m, &[caller.kp.pubkey()]);
        let token = if input_is_a {
            caller.token_a
        } else {
            caller.token_b
        };
        let untouched: Vec<u8> = (0..8).filter(|i| *i != x && *i != y).collect();
        let before: Vec<Vec<u8>> = untouched.iter().map(|&i| lane_bytes(&env, &m, i)).collect();
        let vaults = |env: &Env| -> u128 {
            [x, y]
                .iter()
                .map(|&i| {
                    let a = shard_addrs(&m, i, 0);
                    let v = if input_is_a { a.vault_a } else { a.vault_b };
                    u128::from(env.balance(&v))
                })
                .sum()
        };
        let in_before = vaults(&env);
        let (ax, ay) = arrays(&env, &m, x, y);
        let i = ix_rebalance(
            &m,
            &caller.kp.pubkey(),
            x,
            y,
            token,
            args(input_is_a, amount, ax.len()),
            &ax,
            &ay,
        );
        let kp = caller.kp.insecure_clone();
        let cu = env.send(&[i], &[&kp]).unwrap();
        println!("rebalance input_is_a={input_is_a} cu={cu}");
        assert!(cu < 200_000, "cu {cu}");
        assert_eq!(env.balance(&token), profit, "caller started with nothing");
        assert_eq!(Pool::load(&env.shard_state(&m, lo)).unwrap(), pl);
        assert_eq!(Pool::load(&env.shard_state(&m, hi)).unwrap(), ph);
        assert!(pl.sqrt_price <= ph.sqrt_price, "prices must not cross");
        // The two shards together paid out exactly the profit in that token.
        assert_eq!(vaults(&env), in_before - u128::from(profit));
        let after: Vec<Vec<u8>> = untouched.iter().map(|&i| lane_bytes(&env, &m, i)).collect();
        assert_eq!(before, after, "other shards untouched");
    }
}

#[test]
fn equals_two_plain_swaps() {
    for input_is_a in [false, true] {
        let (mut e1, m1, _) = diverged();
        let (mut e2, m2, _) = diverged();
        let amount = 2_000_000_000;
        let caller = e1.user(&m1, 0);
        e1.set_rebalancers(&m1, &[caller.kp.pubkey()]);
        // Same clock and slot in the twin market.
        e2.set_rebalancers(&m2, &[Pubkey::new_unique()]);
        let token = if input_is_a {
            caller.token_a
        } else {
            caller.token_b
        };
        let (ax, ay) = arrays(&e1, &m1, 0, 1);
        let i = ix_rebalance(
            &m1,
            &caller.kp.pubkey(),
            0,
            1,
            token,
            args(input_is_a, amount, ax.len()),
            &ax,
            &ay,
        );
        let kp = caller.kp.insecure_clone();
        e1.send(&[i], &[&kp]).unwrap();

        // The same two legs as plain swaps by a funded trader.
        let t = e2.user(&m2, 1_000_000_000_000);
        let (lo0, hi0) = (
            e2.shard_state(&m2, 0).sqrt_price_x64,
            e2.shard_state(&m2, 1).sqrt_price_x64,
        );
        let leg = |e: &mut Env, id: u8, a_to_b: bool, amount: u64, limit: u128| -> u64 {
            let mut a = e.swap_args(a_to_b, amount);
            a.sqrt_price_limit_x64 = limit;
            let arrays = e.swap_arrays(&m2, id, a_to_b);
            let i = e.ix_swap(&m2, &t, id, a, &arrays);
            let out_acc = if a_to_b { t.token_b } else { t.token_a };
            let before = e.balance(&out_acc);
            let kp = t.kp.insecure_clone();
            e.send(&[i], &[&kp]).unwrap();
            e.balance(&out_acc) - before
        };
        if input_is_a {
            let mid = leg(&mut e2, 1, true, amount, lo0);
            let hi1 = e2.shard_state(&m2, 1).sqrt_price_x64;
            leg(&mut e2, 0, false, mid, hi1);
        } else {
            let mid = leg(&mut e2, 0, false, amount, hi0);
            let lo1 = e2.shard_state(&m2, 0).sqrt_price_x64;
            leg(&mut e2, 1, true, mid, lo1);
        }
        for id in 0..8 {
            assert_eq!(
                Pool::load(&e1.shard_state(&m1, id)).unwrap(),
                Pool::load(&e2.shard_state(&m2, id)).unwrap(),
                "shard {id}"
            );
            let (a1, a2) = (shard_addrs(&m1, id, 0), shard_addrs(&m2, id, 0));
            assert_eq!(
                (e1.balance(&a1.vault_a), e1.balance(&a1.vault_b)),
                (e2.balance(&a2.vault_a), e2.balance(&a2.vault_b)),
                "vaults of shard {id}"
            );
        }
    }
}

#[test]
fn refused_cases() {
    let (mut env, m, _) = diverged();
    let caller = env.user(&m, 0);
    let kp = caller.kp.insecure_clone();
    let send = |env: &mut Env, x: u8, y: u8, token: Pubkey, a: RebalanceArgs| {
        let (ax, ay) = if x == y {
            let v = env.swap_arrays(&m, x, false);
            (v.clone(), vec![])
        } else {
            arrays(env, &m, x, y)
        };
        let a = RebalanceArgs {
            arrays_x: ax.len() as u8,
            ..a
        };
        let i = ix_rebalance(&m, &kp.pubkey(), x, y, token, a, &ax, &ay);
        env.send(&[i], &[&kp])
    };
    // Only listed rebalancers; the keeper is the only one at first.
    assert_eq!(
        env.market_state(&m).rebalancers[0],
        env.keeper.pubkey(),
        "keeper listed at creation"
    );
    expect_custom(
        send(
            &mut env,
            0,
            1,
            caller.token_b,
            args(false, 2_000_000_000, 0),
        ),
        amm_code(AmmError::NotRebalancer),
    );
    // Not the admin either.
    let admin = env.admin.insecure_clone();
    let i = ix_rebalance(
        &m,
        &admin.pubkey(),
        0,
        1,
        caller.token_b,
        args(false, 2_000_000_000, 0),
        &[],
        &[],
    );
    expect_custom(env.send(&[i], &[&admin]), amm_code(AmmError::NotRebalancer));
    env.set_rebalancers(&m, &[Pubkey::new_unique(), caller.kp.pubkey()]);
    let r = send(&mut env, 0, 0, caller.token_b, args(false, 1_000_000, 0));
    assert!(r.is_err(), "same shard twice");
    // Shards 1 and 2 are at the same price.
    expect_custom(
        send(&mut env, 1, 2, caller.token_b, args(false, 1_000_000, 0)),
        amm_code(AmmError::RebalanceNoPriceGap),
    );
    // Profit floor above what the gap pays.
    expect_custom(
        send(
            &mut env,
            0,
            1,
            caller.token_b,
            RebalanceArgs {
                min_profit: u64::MAX,
                ..args(false, 2_000_000_000, 0)
            },
        ),
        amm_code(AmmError::RebalanceUnprofitable),
    );
    // The profit account must hold the input token.
    expect_custom(
        send(
            &mut env,
            0,
            1,
            caller.token_a,
            args(false, 2_000_000_000, 0),
        ),
        amm_code(AmmError::InvalidTokenAccount),
    );
    // Too big: the first leg would pass the other shard's price.
    expect_custom(
        send(
            &mut env,
            0,
            1,
            caller.token_b,
            args(false, 1_000_000_000_000, 0),
        ),
        amm_code(AmmError::PriceLimitReached),
    );
    // Paused rebalancing; swaps still work.
    let pauser = env.pauser.insecure_clone();
    env.send(
        &[ix_flags(&m, &pauser.pubkey(), PAUSE_REBALANCE)],
        &[&pauser],
    )
    .unwrap();
    expect_custom(
        send(
            &mut env,
            0,
            1,
            caller.token_b,
            args(false, 2_000_000_000, 0),
        ),
        amm_code(AmmError::RebalancePaused),
    );
    let t = env.user(&m, 1_000_000_000);
    env.swap(&m, &t, 3, true, 1_000_000).unwrap();
    // Admin clears; it works again.
    let admin = env.admin.insecure_clone();
    env.send(&[ix_flags(&m, &admin.pubkey(), 0)], &[&admin])
        .unwrap();
    send(
        &mut env,
        0,
        1,
        caller.token_b,
        args(false, 2_000_000_000, 0),
    )
    .unwrap();
    // Taken off the list (after the timelock), it is refused again.
    env.set_rebalancers(&m, &[]);
    expect_custom(
        send(&mut env, 0, 1, caller.token_b, args(false, 1_000, 0)),
        amm_code(AmmError::NotRebalancer),
    );
}

fn ix_flags(m: &Market, authority: &Pubkey, flags: u8) -> solana_instruction::Instruction {
    ix(
        sharded_amm::accounts::SetPaused {
            authority: *authority,
            market: m.market,
        },
        sharded_amm::instruction::SetPauseFlags { flags },
    )
}

#[test]
fn pause_flags_roles_and_exits() {
    let mut env = Env::new();
    let (m, lp) = env.open_seeded_market();
    let (admin, pauser, keeper) = (
        env.admin.insecure_clone(),
        env.pauser.insecure_clone(),
        env.keeper.insecure_clone(),
    );
    expect_custom(
        env.send(&[ix_flags(&m, &keeper.pubkey(), PAUSE_SWAPS)], &[&keeper]),
        amm_code(AmmError::Unauthorized),
    );
    expect_custom(
        env.send(&[ix_flags(&m, &admin.pubkey(), 1 << 3)], &[&admin]),
        amm_code(AmmError::InvalidPauseFlags),
    );
    env.send(
        &[ix_flags(&m, &pauser.pubkey(), PAUSE_SWAPS | PAUSE_DEPOSITS)],
        &[&pauser],
    )
    .unwrap();
    // The pauser cannot lift a pause.
    expect_custom(
        env.send(&[ix_flags(&m, &pauser.pubkey(), PAUSE_SWAPS)], &[&pauser]),
        amm_code(AmmError::Unauthorized),
    );
    let t = env.user(&m, 1_000_000_000);
    expect_custom(
        env.swap(&m, &t, 0, true, 1_000_000),
        amm_code(AmmError::SwapsPaused),
    );
    expect_custom(
        env.add_liquidity(&m, &t, 0, -600, 600, 1_000_000),
        amm_code(AmmError::DepositsPaused),
    );
    // LP exits stay open under every flag.
    let i = env.ix_decrease(&m, &lp, 0, -1200, 1200, 1_000_000);
    let c = env.ix_collect(&m, &lp, 0, -1200, 1200);
    let kp = lp.kp.insecure_clone();
    env.send(&[i, c], &[&kp]).unwrap();
    env.send(&[ix_flags(&m, &admin.pubkey(), 0)], &[&admin])
        .unwrap();
    env.swap(&m, &t, 0, true, 1_000_000).unwrap();
    env.add_liquidity(&m, &t, 0, -600, 600, 1_000_000).unwrap();
}

fn ix_shard_flags(
    m: &Market,
    authority: &Pubkey,
    id: u8,
    flags: u8,
) -> solana_instruction::Instruction {
    ix(
        sharded_amm::accounts::SetShardPauseFlags {
            authority: *authority,
            market: m.market,
            shard: shard_addrs(m, id, 0).shard,
        },
        sharded_amm::instruction::SetShardPauseFlags { flags },
    )
}

/// One shard paused: the other seven keep trading, its LPs can still leave,
/// and the roles are those of the market flags.
#[test]
fn shard_pause_stops_one_shard_only() {
    let (mut env, m, lp) = diverged();
    let (admin, pauser, keeper) = (
        env.admin.insecure_clone(),
        env.pauser.insecure_clone(),
        env.keeper.insecure_clone(),
    );
    expect_custom(
        env.send(
            &[ix_shard_flags(&m, &keeper.pubkey(), 1, PAUSE_SWAPS)],
            &[&keeper],
        ),
        amm_code(AmmError::Unauthorized),
    );
    expect_custom(
        env.send(&[ix_shard_flags(&m, &admin.pubkey(), 1, 1 << 3)], &[&admin]),
        amm_code(AmmError::InvalidPauseFlags),
    );
    let all = PAUSE_SWAPS | PAUSE_DEPOSITS | PAUSE_REBALANCE;
    env.send(&[ix_shard_flags(&m, &pauser.pubkey(), 1, all)], &[&pauser])
        .unwrap();
    expect_custom(
        env.send(&[ix_shard_flags(&m, &pauser.pubkey(), 1, 0)], &[&pauser]),
        amm_code(AmmError::Unauthorized),
    );
    assert_eq!(env.shard_state(&m, 1).pause_flags, all);
    assert_eq!(env.market_state(&m).pause_flags, 0, "market untouched");
    let others: Vec<_> = (2..8u8)
        .map(|id| env.raw(&shard_addrs(&m, id, 0).shard).unwrap().data)
        .collect();

    let t = env.user(&m, 1_000_000_000);
    expect_custom(
        env.swap(&m, &t, 1, true, 1_000_000),
        amm_code(AmmError::ShardPaused),
    );
    expect_custom(
        env.add_liquidity(&m, &t, 1, -600, 600, 1_000_000),
        amm_code(AmmError::ShardPaused),
    );
    // Rebalancing through shard 1 is refused, through shard 2 it works.
    let caller = env.user(&m, 0);
    env.set_rebalancers(&m, &[caller.kp.pubkey()]);
    let kp = caller.kp.insecure_clone();
    for (y, ok) in [(1u8, false), (2, true)] {
        let (ax, ay) = arrays(&env, &m, 0, y);
        let a = args(false, 2_000_000_000, ax.len());
        let i = ix_rebalance(&m, &kp.pubkey(), 0, y, caller.token_b, a, &ax, &ay);
        let r = env.send(&[i], &[&kp]);
        if ok {
            r.unwrap();
        } else {
            expect_custom(r, amm_code(AmmError::ShardPaused));
        }
    }
    // Shards 3..7 were not written by anything above; 2 was rebalanced.
    for (id, before) in (3..8u8).zip(&others[1..]) {
        assert_eq!(
            &env.raw(&shard_addrs(&m, id, 0).shard).unwrap().data,
            before,
            "shard {id}"
        );
    }
    env.swap(&m, &t, 4, true, 1_000_000).unwrap();
    env.add_liquidity(&m, &t, 4, -600, 600, 1_000_000).unwrap();
    // LP exits on the paused shard stay open.
    let i = env.ix_decrease(&m, &lp, 1, -1200, 1200, 1_000_000);
    let c = env.ix_collect(&m, &lp, 1, -1200, 1200);
    let lkp = lp.kp.insecure_clone();
    env.send(&[i, c], &[&lkp]).unwrap();
    // Lifting the swap pause needs reopen_shard and its price check.
    expect_custom(
        env.send(&[ix_shard_flags(&m, &admin.pubkey(), 1, 0)], &[&admin]),
        amm_code(AmmError::ReopenNeedsPriceCheck),
    );
    env.send(
        &[ix_reopen(&m, &admin.pubkey(), 1, 0, &[3, 4, 5])],
        &[&admin],
    )
    .unwrap();
    env.swap(&m, &t, 1, true, 1_000_000).unwrap();
    env.add_liquidity(&m, &t, 1, -600, 600, 1_000_000).unwrap();
}

fn ix_reopen(
    m: &Market,
    admin: &Pubkey,
    id: u8,
    flags: u8,
    refs: &[u8],
) -> solana_instruction::Instruction {
    let mut i = ix(
        sharded_amm::accounts::ReopenShard {
            admin: *admin,
            market: m.market,
            registry: m.registry,
            shard: shard_addrs(m, id, 0).shard,
        },
        sharded_amm::instruction::ReopenShard { flags },
    );
    i.accounts.extend(refs.iter().map(|r| {
        solana_instruction::AccountMeta::new_readonly(shard_addrs(m, *r, 0).shard, false)
    }));
    i
}

/// Shard 0's price is more than 1% off the others when its swaps are
/// paused. It cannot reopen until the listed rebalancer, trading against an
/// open shard, brings it back; then `reopen_shard` checks the median of the
/// references in the same transaction.
#[test]
fn controlled_reopen_after_recovery() {
    let (mut env, m, _) = diverged();
    let (admin, pauser) = (env.admin.insecure_clone(), env.pauser.insecure_clone());
    env.send(
        &[ix_shard_flags(&m, &pauser.pubkey(), 0, PAUSE_SWAPS)],
        &[&pauser],
    )
    .unwrap();
    let t = env.user(&m, 1_000_000_000_000);
    expect_custom(
        env.swap(&m, &t, 0, true, 1_000_000),
        amm_code(AmmError::ShardPaused),
    );
    let reopen = |env: &mut Env, refs: &[u8]| {
        env.send(&[ix_reopen(&m, &admin.pubkey(), 0, 0, refs)], &[&admin])
    };
    expect_custom(
        reopen(&mut env, &[1, 2, 3]),
        amm_code(AmmError::PriceDeviationTooHigh),
    );
    expect_custom(
        reopen(&mut env, &[1, 2]),
        amm_code(AmmError::InvalidAccountCount),
    );
    expect_custom(
        reopen(&mut env, &[0, 1, 2]),
        amm_code(AmmError::DuplicateAccount),
    );
    expect_custom(
        reopen(&mut env, &[1, 1, 2]),
        amm_code(AmmError::DuplicateAccount),
    );
    // Only the admin.
    let i = ix_reopen(&m, &pauser.pubkey(), 0, 0, &[1, 2, 3]);
    assert!(env.send(&[i], &[&pauser]).is_err());

    // Recovery: the listed rebalancer, against an open shard only.
    env.send(
        &[ix_shard_flags(&m, &pauser.pubkey(), 4, PAUSE_SWAPS)],
        &[&pauser],
    )
    .unwrap();
    expect_custom(
        reopen(&mut env, &[1, 2, 4]),
        amm_code(AmmError::ShardPaused),
    );
    let caller = env.user(&m, 0);
    env.set_rebalancers(&m, &[caller.kp.pubkey()]);
    let kp = caller.kp.insecure_clone();
    let rebalance = |env: &mut Env, y: u8, amount: u64| {
        let (ax, ay) = arrays(env, &m, 0, y);
        let a = args(false, amount, ax.len());
        let i = ix_rebalance(&m, &kp.pubkey(), 0, y, caller.token_b, a, &ax, &ay);
        env.send(&[i], &[&kp])
    };
    expect_custom(
        rebalance(&mut env, 4, 1_000_000),
        amm_code(AmmError::ShardPaused),
    );
    // The bot trades shard 0 against each open shard in turn, each time
    // with the largest amount that still fits (the program refuses one that
    // would push the prices past each other).
    let before = env.shard_state(&m, 0).sqrt_price_x64;
    let mut rounds = 0;
    for y in [1u8, 2, 3, 5, 6, 7, 1, 2, 3, 5, 6, 7] {
        rounds += usize::from(
            (20..40)
                .rev()
                .any(|k| rebalance(&mut env, y, 1u64 << k).is_ok()),
        );
    }
    assert!(rounds > 0, "a recovery rebalance fits");
    assert!(env.shard_state(&m, 0).sqrt_price_x64 > before);
    assert!(env.balance(&caller.token_b) > 0, "paid only its profit");

    // Back within the deviation of the median: reopens, swaps work.
    reopen(&mut env, &[1, 2, 3]).unwrap();
    assert_eq!(env.shard_state(&m, 0).pause_flags, 0);
    env.swap(&m, &t, 0, true, 1_000_000).unwrap();
    // Nothing left to reopen.
    expect_custom(
        reopen(&mut env, &[1, 2, 3]),
        amm_code(AmmError::NoPendingChange),
    );
}
