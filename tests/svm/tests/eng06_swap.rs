//! ENG-06: swaps on-chain against the host engine.
//!
//! The differential test replays a random workload (swaps in both
//! directions, liquidity added and removed, fees collected) and, before every
//! swap, runs the same `engine::swap` on a copy of the shard and its tick
//! arrays read from the SVM. The on-chain result must match bit for bit: the
//! same error code, or the same shard state, tick arrays and token deltas.
//! Token conservation over every known account is checked after each step,
//! and at the end every LP and the revenue authority withdraw everything.
//!
//! `AMM_SVM_CASES` (default 400) and `AMM_SVM_SEED` scale the workload.

mod common;

use common::*;
use sharded_amm::engine::{dynamic_fee, swap, Pool, SwapOutcome, SwapParams};
use sharded_amm::errors::AmmError;
use sharded_amm::math::tick_math::{sqrt_price_at_tick, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};
use sharded_amm::state::{array_span, ShardState, TickArray};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction_error::TransactionError;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Log-uniform in [1, 2^bits).
    fn log_amount(&mut self, bits: u32) -> u64 {
        let b = 1 + self.below(bits as u64) as u32;
        (self.next() >> (64 - b)).max(1)
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn custom_code(e: &TransactionError) -> Option<u32> {
    match e {
        TransactionError::InstructionError(_, InstructionError::Custom(c)) => Some(*c),
        _ => None,
    }
}

fn compute_budget(units: u32) -> solana_instruction::Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    solana_instruction::Instruction {
        program_id: anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data,
    }
}

struct Prediction {
    out: SwapOutcome,
    pool: Pool,
    fee_state: sharded_amm::state::DynamicFeeState,
    arrays: Vec<TickArray>,
}

/// Runs the host engine on copies of the on-chain shard and arrays.
fn predict(
    env: &Env,
    m: &Market,
    id: u8,
    args: &sharded_amm::instructions::SwapArgs,
    arrays: &[Pubkey],
) -> Result<Prediction, AmmError> {
    let market = env.market_state(m);
    let params = market.params;
    let st: ShardState = env.shard_state(m, id);
    let mut pool = Pool::load(&st)?;
    let slot = env.svm.get_sysvar::<anchor_lang::prelude::Clock>().slot;
    let mut fee_state = st.dynamic_fee;
    let fee_rate_ppm = dynamic_fee::begin(
        &mut fee_state,
        &market.dynamic_fees,
        params.fee_rate_ppm,
        params.tick_spacing,
        pool.tick,
        slot,
    );
    if arrays.is_empty() || arrays.len() > 3 {
        return Err(AmmError::InvalidAccountCount);
    }
    let mut copies: Vec<TickArray> = arrays.iter().map(|k| env.tick_array(k)).collect();
    let mut refs: Vec<&mut TickArray> = copies.iter_mut().collect();
    let out = swap(
        &mut pool,
        &mut refs,
        SwapParams {
            a_to_b: args.a_to_b,
            amount_in: args.amount_in,
            sqrt_price_limit: args.sqrt_price_limit_x64,
            fee_rate_ppm,
            protocol_share_bps: params.protocol_share_bps,
            tick_spacing: params.tick_spacing,
        },
    )?;
    dynamic_fee::finish(
        &mut fee_state,
        &market.dynamic_fees,
        params.tick_spacing,
        pool.tick,
        slot,
    );
    if out.amount_out < args.min_amount_out {
        return Err(AmmError::SlippageExceeded);
    }
    if out.rounding_dust > args.max_rounding_dust {
        return Err(AmmError::RoundingDustExceeded);
    }
    Ok(Prediction {
        out,
        pool,
        fee_state,
        arrays: copies,
    })
}

#[derive(Debug)]
struct Checked {
    out: SwapOutcome,
    cu: u64,
}

/// Sends a swap and asserts it matches the host prediction exactly.
fn checked_swap(
    env: &mut Env,
    m: &Market,
    user: &Lp,
    id: u8,
    args: sharded_amm::instructions::SwapArgs,
    arrays: &[Pubkey],
) -> Result<Checked, AmmError> {
    let expected = predict(env, m, id, &args, arrays);
    let (tin, tout) = if args.a_to_b {
        (user.token_a, user.token_b)
    } else {
        (user.token_b, user.token_a)
    };
    let before = (env.balance(&tin), env.balance(&tout));
    let st_before = env.shard_state(m, id);
    let arrays_before: Vec<TickArray> = arrays.iter().map(|k| env.tick_array(k)).collect();
    let i = env.ix_swap(m, user, id, args, arrays);
    let kp = user.kp.insecure_clone();
    // Long random swaps can cross many ticks; ask for the transaction maximum
    // so the comparison is about results. CU is asserted separately.
    let got = env.send(&[compute_budget(1_400_000), i], &[&kp]);
    match (expected, got) {
        (Ok(p), Ok(cu)) => {
            let after = (env.balance(&tin), env.balance(&tout));
            assert_eq!(before.0 - after.0, p.out.amount_in, "input debited");
            assert_eq!(after.1 - before.1, p.out.amount_out, "output credited");
            assert_eq!(p.out.amount_in, args.amount_in, "no partial fills");
            let st = env.shard_state(m, id);
            assert_eq!(Pool::load(&st).unwrap(), p.pool, "shard state");
            assert_eq!(st.dynamic_fee, p.fee_state, "dynamic fee state");
            for (k, want) in arrays.iter().zip(&p.arrays) {
                assert_eq!(&env.tick_array(k), want, "tick array {k}");
            }
            Ok(Checked { out: p.out, cu })
        }
        (Err(e), Err(t)) => {
            assert_eq!(
                custom_code(&t),
                Some(amm_code(e)),
                "on-chain error {t:?}, host {e:?}"
            );
            assert_eq!(env.shard_state(m, id), st_before, "failed swap left state");
            for (k, want) in arrays.iter().zip(&arrays_before) {
                assert_eq!(&env.tick_array(k), want, "failed swap left array");
            }
            assert_eq!((env.balance(&tin), env.balance(&tout)), before);
            Err(e)
        }
        (Ok(p), Err(t)) => panic!("host ok {:?}, on-chain failed {t:?}", p.out),
        (Err(e), Ok(_)) => panic!("host failed {e:?}, on-chain succeeded"),
    }
}

/// Sum of balances per mint over every token account the test created.
fn totals(env: &Env, m: &Market, accounts: &[Pubkey]) -> (u128, u128) {
    let mut t = (0u128, 0u128);
    for k in accounts {
        let (mint, _, amount) = token_account_fields(&env.raw(k).unwrap().data);
        if mint == m.mint_a {
            t.0 += amount as u128;
        } else {
            assert_eq!(mint, m.mint_b);
            t.1 += amount as u128;
        }
    }
    t
}

struct Pos {
    lp: usize,
    id: u8,
    lower: i32,
    upper: i32,
    liquidity: u128,
}

#[test]
fn random_workload_matches_host_engine_and_conserves_tokens() {
    let cases = env_u64("AMM_SVM_CASES", 400);
    let seed = env_u64("AMM_SVM_SEED", 0x5eed_0006);
    let mut rng = Rng(seed);
    let mut env = Env::new();
    let (m, seed_lp) = env.open_seeded_market();
    // Even seeds (the default included) run with the dynamic fee on
    // (ADR-0005), so the host engine and the program must also agree on the
    // fee from swap to swap; odd seeds keep the fixed fee.
    let dynamic = seed.is_multiple_of(2);
    if dynamic {
        let admin = env.admin.insecure_clone();
        let base = env.market_state(&m).params.fee_rate_ppm;
        let p = sharded_amm::state::DynamicFeeParams {
            max_fee_rate_ppm: sharded_amm::constants::MAX_DYNAMIC_FEE_RATE_PPM,
            vol_coeff_ppm: 1 + rng.below(500) as u32,
            load_coeff_ppm: rng.below(300) as u32,
            max_vol_accumulator: 1 + rng.below(1_000_000) as u32,
            filter_slots: 1 + rng.below(10) as u16,
            decay_slots: 20 + rng.below(100) as u16,
            reduction_bps: rng.below(10_001) as u16,
        };
        env.send(
            &[Env::ix_update_fees_dynamic(
                &m,
                &admin.pubkey(),
                base,
                2_000,
                p,
            )],
            &[&admin],
        )
        .unwrap();
        env.warp(sharded_amm::constants::FEE_TIMELOCK_SECONDS);
        env.send(&[Env::ix_apply_fees(&m, &admin.pubkey())], &[&admin])
            .unwrap();
    }
    println!("dynamic fee: {dynamic}");
    let spacing = env.market_state(&m).params.tick_spacing as i32;
    let span = array_span(spacing as u16);
    // Arrays across +-4 spans on every shard so swaps can travel and cross.
    for id in 0..8 {
        env.ensure_arrays(&m, id, -4 * span, 4 * span - 1);
    }
    let lps: Vec<Lp> = (0..4).map(|_| env.user(&m, u64::MAX / 8)).collect();
    let traders: Vec<Lp> = (0..3).map(|_| env.user(&m, u64::MAX / 8)).collect();
    let revenue = env.revenue.insecure_clone();
    let rev_a = env.token_account(&m.mint_a, &revenue.pubkey(), 0);
    let rev_b = env.token_account(&m.mint_b, &revenue.pubkey(), 0);
    let mut accounts = vec![seed_lp.token_a, seed_lp.token_b, rev_a, rev_b];
    for u in lps.iter().chain(&traders) {
        accounts.extend([u.token_a, u.token_b]);
    }
    for id in 0..8 {
        let a = shard_addrs(&m, id, 0);
        accounts.extend([a.vault_a, a.vault_b]);
    }
    let total0 = totals(&env, &m, &accounts);

    let mut positions: Vec<Pos> = Vec::new();
    // A thin position over the whole array window on every shard, so swaps
    // mostly cross ticks instead of stopping at a liquidity gap.
    for id in 0..8 {
        let (lower, upper) = (-4 * span, 4 * span - spacing);
        let liquidity = 1u128 << (30 + rng.below(12));
        env.add_liquidity(&m, &lps[0], id, lower, upper, liquidity)
            .unwrap();
        positions.push(Pos {
            lp: 0,
            id,
            lower,
            upper,
            liquidity,
        });
        // And a dozen narrower ones, so there are many ticks to cross.
        for k in 0..12 {
            let lp = 1 + (k % (lps.len() - 1));
            let c = rng.below(200) as i32 - 100;
            let w = 1 + rng.below(20) as i32;
            let (lower, upper) = ((c - w) * spacing, (c + w) * spacing);
            if lower < -4 * span || upper > 4 * span - spacing {
                continue;
            }
            let liquidity = 1u128 << (28 + rng.below(16));
            if positions
                .iter()
                .any(|p| p.lp == lp && p.id == id && p.lower == lower && p.upper == upper)
            {
                continue;
            }
            env.add_liquidity(&m, &lps[lp], id, lower, upper, liquidity)
                .unwrap();
            positions.push(Pos {
                lp,
                id,
                lower,
                upper,
                liquidity,
            });
        }
    }
    let (mut ok, mut failed, mut crossed, mut max_cu) = (0u64, 0u64, 0u64, 0u64);
    let mut errors = std::collections::BTreeMap::<String, u64>::new();
    for step in 0..cases {
        let id = rng.below(8) as u8;
        match rng.below(100) {
            0..=69 => {
                let u = &traders[rng.below(traders.len() as u64) as usize];
                let a_to_b = rng.below(2) == 0;
                // Sized against the active liquidity: input ~ L * 2^-u moves
                // the sqrt price by about 2^-u, from a fraction of a tick
                // (u = 24) to many crossings (u = 2).
                let l = env.shard_state(&m, id).liquidity;
                let amount = if l == 0 || rng.below(8) == 0 {
                    rng.log_amount(46)
                } else {
                    let u = 2 + rng.below(23) as u32;
                    ((l >> u) as u64).saturating_add(rng.below(1000)).max(1)
                };
                let mut args = env.swap_args(a_to_b, amount);
                // Sometimes a price limit, sometimes on the wrong side.
                if rng.below(5) == 0 {
                    let t = env.shard_state(&m, id).tick_current;
                    let d = (rng.below(3000) as i32 - 600) * if a_to_b { -1 } else { 1 };
                    let lt = (t + d).clamp(-443_000, 443_000);
                    args.sqrt_price_limit_x64 = sqrt_price_at_tick(lt).unwrap();
                }
                if rng.below(10) == 0 {
                    args.min_amount_out = rng.log_amount(40);
                }
                if rng.below(10) == 0 {
                    args.max_rounding_dust = rng.below(2);
                }
                // Slots advance by 0..=3 between swaps: some share a slot.
                let mut clock = env.svm.get_sysvar::<anchor_lang::prelude::Clock>();
                clock.slot += rng.below(4);
                env.svm.set_sysvar(&clock);
                let arrays = env.swap_arrays(&m, id, a_to_b);
                match checked_swap(&mut env, &m, u, id, args, &arrays) {
                    Ok(c) => {
                        ok += 1;
                        crossed += c.out.ticks_crossed as u64;
                        max_cu = max_cu.max(c.cu);
                    }
                    Err(e) => {
                        failed += 1;
                        *errors.entry(format!("{e:?}")).or_default() += 1;
                    }
                }
            }
            70..=84 => {
                let lp = rng.below(lps.len() as u64) as usize;
                let w = 1 + rng.below(40) as i32;
                let c = rng.below(160) as i32 - 80;
                let (lower, upper) = ((c - w) * spacing, (c + w) * spacing);
                let lower = lower.max(-4 * span);
                let upper = upper.min(4 * span - spacing);
                if lower >= upper {
                    continue;
                }
                let liquidity = rng.log_amount(44) as u128;
                if env
                    .add_liquidity(&m, &lps[lp], id, lower, upper, liquidity)
                    .is_ok()
                {
                    match positions
                        .iter_mut()
                        .find(|p| p.lp == lp && p.id == id && p.lower == lower && p.upper == upper)
                    {
                        Some(p) => p.liquidity += liquidity,
                        None => positions.push(Pos {
                            lp,
                            id,
                            lower,
                            upper,
                            liquidity,
                        }),
                    }
                }
            }
            85..=94 if !positions.is_empty() => {
                let k = rng.below(positions.len() as u64) as usize;
                let p = &mut positions[k];
                if p.liquidity == 0 {
                    continue;
                }
                let amount = 1 + (rng.next() as u128 % p.liquidity);
                let i = env.ix_decrease(&m, &lps[p.lp], p.id, p.lower, p.upper, amount);
                let kp = lps[p.lp].kp.insecure_clone();
                env.send(&[i], &[&kp]).expect("decrease within position");
                p.liquidity -= amount;
            }
            95..=97 if !positions.is_empty() => {
                let p = &positions[rng.below(positions.len() as u64) as usize];
                let i = env.ix_collect(&m, &lps[p.lp], p.id, p.lower, p.upper);
                let kp = lps[p.lp].kp.insecure_clone();
                env.send(&[i], &[&kp]).expect("collect");
            }
            _ => {
                let st = env.shard_state(&m, id);
                let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), id, rev_a, rev_b);
                let r = env.send(&[i], &[&revenue]);
                if st.protocol_fees_a > 0 || st.protocol_fees_b > 0 {
                    r.expect("collect protocol");
                    let after = env.shard_state(&m, id);
                    assert_eq!((after.protocol_fees_a, after.protocol_fees_b), (0, 0));
                } else {
                    expect_custom(r, amm_code(AmmError::ZeroAmount));
                }
            }
        }
        assert_eq!(
            totals(&env, &m, &accounts),
            total0,
            "conservation at step {step}"
        );
    }

    // Everyone leaves: all liquidity, all LP fees, all protocol fees.
    for p in &positions {
        let kp = lps[p.lp].kp.insecure_clone();
        if p.liquidity > 0 {
            let i = env.ix_decrease(&m, &lps[p.lp], p.id, p.lower, p.upper, p.liquidity);
            env.send(&[i], &[&kp]).expect("final decrease");
        }
        let i = env.ix_collect(&m, &lps[p.lp], p.id, p.lower, p.upper);
        env.send(&[i], &[&kp]).expect("final collect");
    }
    for id in 0..8 {
        let i = env.ix_decrease(&m, &seed_lp, id, -1200, 1200, 1_000_000_000_000);
        let kp = seed_lp.kp.insecure_clone();
        env.send(&[i], &[&kp]).expect("seed exit");
        let i = env.ix_collect(&m, &seed_lp, id, -1200, 1200);
        env.send(&[i], &[&kp]).expect("seed collect");
        let st = env.shard_state(&m, id);
        assert_eq!(st.liquidity, 0, "shard {id} empty");
        if st.protocol_fees_a > 0 || st.protocol_fees_b > 0 {
            let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), id, rev_a, rev_b);
            env.send(&[i], &[&revenue]).expect("final protocol collect");
        }
    }
    assert_eq!(totals(&env, &m, &accounts), total0);
    let mut dust = (0u64, 0u64);
    for id in 0..8 {
        let a = shard_addrs(&m, id, 0);
        dust.0 += env.balance(&a.vault_a);
        dust.1 += env.balance(&a.vault_b);
    }
    println!(
        "STATS swaps_ok={ok} swaps_failed={failed} ticks_crossed={crossed} max_swap_cu={max_cu} \
         positions={} vault_dust={dust:?} errors={errors:?}",
        positions.len()
    );
    assert!(ok > cases / 4, "workload too easy to fail");
    assert!(crossed > cases / 5, "too few ticks crossed: {crossed}");
    // Rounding favors the pool by a few units per operation, never more.
    assert!(dust.0 + dust.1 <= 8 * cases, "vault dust {dust:?}");
    assert!(max_cu < 1_400_000);
}

#[test]
fn swap_guards_slippage_dust_deadline_limits() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000_000);
    let arrays = env.swap_arrays(&m, 0, true);

    let args = env.swap_args(true, 50_000_000);
    let p = predict(&env, &m, 0, &args, &arrays).unwrap();
    // min_amount_out one above the exact output fails, the exact output passes.
    let mut a = args;
    a.min_amount_out = p.out.amount_out + 1;
    assert_eq!(
        checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap_err(),
        AmmError::SlippageExceeded
    );
    a.min_amount_out = p.out.amount_out;
    checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap();

    // Dust bound: find a swap with dust > 0, then cap it one below.
    let mut found = false;
    for amount in 1_000u64..1_200 {
        let a = env.swap_args(false, amount);
        let arrays = env.swap_arrays(&m, 0, false);
        let p = predict(&env, &m, 0, &a, &arrays).unwrap();
        if p.out.rounding_dust > 0 {
            let mut b = a;
            b.max_rounding_dust = p.out.rounding_dust - 1;
            assert_eq!(
                checked_swap(&mut env, &m, &t, 0, b, &arrays).unwrap_err(),
                AmmError::RoundingDustExceeded
            );
            found = true;
            break;
        }
    }
    assert!(found, "no swap with rounding dust found");

    // Deadline in the past.
    let mut a = env.swap_args(true, 1_000);
    a.deadline = env.now() - 1;
    let i = env.ix_swap(&m, &t, 0, a, &arrays);
    expect_custom(
        env.send(&[i], &[&t.kp]),
        amm_code(AmmError::DeadlineExceeded),
    );

    // Zero input.
    let a = env.swap_args(true, 0);
    assert_eq!(
        checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap_err(),
        AmmError::ZeroAmount
    );

    // Limits: on the wrong side, out of domain, and reached (no partial fill).
    let price = env.shard_state(&m, 0).sqrt_price_x64;
    for (a_to_b, limit) in [
        (true, price + 1),
        (true, price),
        (false, price - 1),
        (true, MIN_SQRT_PRICE_X64),
        (false, MAX_SQRT_PRICE_X64),
        (false, u128::MAX),
    ] {
        let mut a = env.swap_args(a_to_b, 1_000);
        a.sqrt_price_limit_x64 = limit;
        let arrays = env.swap_arrays(&m, 0, a_to_b);
        let e = checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap_err();
        assert_eq!(
            e,
            AmmError::InvalidPriceLimit,
            "limit {limit} a_to_b {a_to_b}"
        );
    }
    let mut a = env.swap_args(true, 1_000_000_000_000);
    a.sqrt_price_limit_x64 = sqrt_price_at_tick(-30).unwrap();
    let arrays = env.swap_arrays(&m, 0, true);
    assert_eq!(
        checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap_err(),
        AmmError::PriceLimitReached
    );
    // Past the seeded range there is no liquidity: a gap, not a partial fill.
    let a = env.swap_args(true, u64::MAX / 4);
    assert!(matches!(
        checked_swap(&mut env, &m, &t, 0, a, &arrays).unwrap_err(),
        AmmError::ZeroLiquidityGap | AmmError::TickArraysExhausted
    ));
}

#[test]
fn swap_rejects_wrong_accounts() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let span = array_span(60);
    env.ensure_arrays(&m, 0, -4 * span, 4 * span - 1);
    env.ensure_arrays(&m, 1, -4 * span, 4 * span - 1);
    let t = env.user(&m, 1_000_000_000_000);
    let args = env.swap_args(true, 1_000_000);
    let arrays = env.swap_arrays(&m, 0, true);
    assert_eq!(arrays.len(), 3);
    let send = |env: &mut Env, i| {
        let kp = t.kp.insecure_clone();
        env.send(&[i], &[&kp])
    };

    // Token accounts swapped (wrong mints for the direction).
    let mut i = env.ix_swap(&m, &t, 0, args, &arrays);
    i.accounts[6].pubkey = t.token_b;
    i.accounts[7].pubkey = t.token_a;
    expect_custom(send(&mut env, i), amm_code(AmmError::InvalidTokenAccount));
    // Another shard's vault.
    let other = shard_addrs(&m, 1, 0);
    let mut i = env.ix_swap(&m, &t, 0, args, &arrays);
    i.accounts[4].pubkey = other.vault_a;
    expect_custom(send(&mut env, i), anchor_codes::CONSTRAINT_HAS_ONE);
    // Another shard's vault authority.
    let mut i = env.ix_swap(&m, &t, 0, args, &arrays);
    i.accounts[8].pubkey = other.vault_authority;
    expect_custom(send(&mut env, i), anchor_codes::CONSTRAINT_SEEDS);
    // Paying out to an account the user does not own still needs the user's
    // input; a thief's output account is fine for the pool, so only check
    // that the user's own input account must be signed for.
    let thief = env.user(&m, 0);
    let mut i = env.ix_swap(&m, &t, 0, args, &arrays);
    i.accounts[6].pubkey = thief.token_a;
    assert!(send(&mut env, i).is_err());

    // Tick arrays: none, too many, duplicate, wrong order, wrong shard,
    // read-only, first not containing the current tick.
    let i = env.ix_swap(&m, &t, 0, args, &[]);
    expect_custom(send(&mut env, i), amm_code(AmmError::InvalidAccountCount));
    let mut four = arrays.clone();
    four.push(tick_array_pda(&shard_addrs(&m, 0, 0).shard, -4 * span));
    let i = env.ix_swap(&m, &t, 0, args, &four);
    expect_custom(send(&mut env, i), amm_code(AmmError::InvalidAccountCount));
    let i = env.ix_swap(&m, &t, 0, args, &[arrays[0], arrays[0]]);
    expect_custom(send(&mut env, i), amm_code(AmmError::DuplicateAccount));
    let i = env.ix_swap(&m, &t, 0, args, &[arrays[0], arrays[2], arrays[1]]);
    expect_custom(send(&mut env, i), amm_code(AmmError::TickArrayMismatch));
    let i = env.ix_swap(&m, &t, 0, args, &[arrays[1], arrays[0]]);
    expect_custom(send(&mut env, i), amm_code(AmmError::TickArrayMismatch));
    let foreign = env.swap_arrays(&m, 1, true);
    let i = env.ix_swap(&m, &t, 0, args, &foreign);
    expect_custom(send(&mut env, i), amm_code(AmmError::TickArrayMismatch));
    let mut i = env.ix_swap(&m, &t, 0, args, &arrays);
    let n = i.accounts.len();
    i.accounts[n - 3].is_writable = false;
    expect_custom(send(&mut env, i), amm_code(AmmError::TickArrayMismatch));
    // A random account as a tick array.
    let i = env.ix_swap(&m, &t, 0, args, &[t.token_a]);
    assert!(send(&mut env, i).is_err());

    // The swap works with the right accounts. From tick 0 moving down the
    // first array is left at once, so two arrays are the minimum here.
    let i = env.ix_swap(&m, &t, 0, args, &arrays[..2]);
    send(&mut env, i).unwrap();
}

#[test]
fn protocol_fees_go_only_to_the_revenue_authority() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let t = env.user(&m, 1_000_000_000_000);
    for k in 0..6 {
        env.swap(&m, &t, 0, k % 2 == 0, 300_000_000).unwrap();
    }
    let st = env.shard_state(&m, 0);
    assert!(st.protocol_fees_a > 0 && st.protocol_fees_b > 0);
    let revenue = env.revenue.insecure_clone();
    let admin = env.admin.insecure_clone();
    let rev_a = env.token_account(&m.mint_a, &revenue.pubkey(), 0);
    let rev_b = env.token_account(&m.mint_b, &revenue.pubkey(), 0);
    let adm_a = env.token_account(&m.mint_a, &admin.pubkey(), 0);
    let adm_b = env.token_account(&m.mint_b, &admin.pubkey(), 0);

    // Wrong signer, even with its own destinations.
    let i = Env::ix_collect_protocol(&m, &admin.pubkey(), 0, adm_a, adm_b);
    expect_custom(env.send(&[i], &[&admin]), anchor_codes::CONSTRAINT_HAS_ONE);
    // Right signer, destination owned by someone else.
    let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), 0, adm_a, rev_b);
    assert!(env.send(&[i], &[&revenue]).is_err());
    // Mints crossed.
    let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), 0, rev_b, rev_a);
    assert!(env.send(&[i], &[&revenue]).is_err());
    // Another shard's vault cannot be drained through shard 0.
    let mut i = Env::ix_collect_protocol(&m, &revenue.pubkey(), 0, rev_a, rev_b);
    i.accounts[3].pubkey = shard_addrs(&m, 1, 0).vault_a;
    expect_custom(
        env.send(&[i], &[&revenue]),
        anchor_codes::CONSTRAINT_HAS_ONE,
    );

    let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), 0, rev_a, rev_b);
    env.send(&[i], &[&revenue]).unwrap();
    assert_eq!(env.balance(&rev_a), st.protocol_fees_a);
    assert_eq!(env.balance(&rev_b), st.protocol_fees_b);
    let after = env.shard_state(&m, 0);
    assert_eq!((after.protocol_fees_a, after.protocol_fees_b), (0, 0));
    // Nothing left to collect.
    let i = Env::ix_collect_protocol(&m, &revenue.pubkey(), 0, rev_a, rev_b);
    expect_custom(env.send(&[i], &[&revenue]), amm_code(AmmError::ZeroAmount));
}

#[test]
fn lp_fees_are_pro_rata_and_protocol_share_is_exact() {
    let mut env = Env::new();
    let m = env.market();
    let keeper = env.keeper.insecure_clone();
    let small = env.user(&m, u64::MAX / 4);
    let big = env.user(&m, u64::MAX / 4);
    for id in 0..8 {
        env.prepare(&m, &keeper, id).unwrap();
        env.send(
            &[Env::ix_initialize_price(&m, &keeper.pubkey(), id, Q64)],
            &[&keeper],
        )
        .unwrap();
        env.add_liquidity(&m, &small, id, -600, 600, 1_000_000_000_000)
            .unwrap();
        env.add_liquidity(&m, &big, id, -600, 600, 3_000_000_000_000)
            .unwrap();
    }
    let i = env.ix_open_market(&m, &keeper.pubkey(), &(0..8).collect::<Vec<_>>());
    env.send(&[i], &[&keeper]).unwrap();
    let t = env.user(&m, 1_000_000_000_000);
    let mut fee_a = 0u128;
    let mut proto_a = 0u128;
    for _ in 0..20 {
        let arrays = env.swap_arrays(&m, 0, true);
        let args = env.swap_args(true, 777_777_777);
        let c = checked_swap(&mut env, &m, &t, 0, args, &arrays).unwrap();
        fee_a += c.out.total_fee as u128;
        proto_a += c.out.protocol_fee as u128;
        let arrays = env.swap_arrays(&m, 0, false);
        let args = env.swap_args(false, 777_777_777);
        checked_swap(&mut env, &m, &t, 0, args, &arrays).unwrap();
    }
    // Protocol gets floor(total * 2000 / 10000) per swap; summed it is within
    // one unit per swap of 20%.
    let want = fee_a * 2_000 / 10_000;
    assert!(
        proto_a <= want && want - proto_a <= 20,
        "protocol {proto_a} vs {want}"
    );

    let before = (env.balance(&small.token_a), env.balance(&big.token_a));
    for lp in [&small, &big] {
        let i = env.ix_collect(&m, lp, 0, -600, 600);
        env.send(&[i], &[&lp.kp]).unwrap();
    }
    let got_small = (env.balance(&small.token_a) - before.0) as u128;
    let got_big = (env.balance(&big.token_a) - before.1) as u128;
    assert!(got_small > 0);
    // 1:3 within rounding, and LP + protocol never exceeds the fees charged.
    assert!(
        got_big.abs_diff(3 * got_small) <= 3,
        "{got_small} vs {got_big}"
    );
    assert!(got_small + got_big + proto_a <= fee_a);
    assert!(fee_a - (got_small + got_big + proto_a) <= 40);
}

#[test]
fn swap_compute_units_per_step_and_crossing() {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market();
    let span = array_span(60);
    env.ensure_arrays(&m, 0, -3 * span, 3 * span - 1);
    let lp = env.user(&m, u64::MAX / 4);
    // A position boundary on every spacing step within +-1 array span.
    for k in 1..32 {
        env.add_liquidity(&m, &lp, 0, -60 * k, 60 * k, 10_000_000_000)
            .unwrap();
    }
    let t = env.user(&m, u64::MAX / 4);
    // One step, inside a tick interval, under the default 200k budget.
    let cu = env.swap(&m, &t, 0, true, 1_000).unwrap();
    println!("single step swap cu={cu}");
    assert!(cu < 50_000, "single step {cu}");
    // Cross about ten initialized ticks under the default budget.
    let mut lo = 0u64;
    let mut hi = 1u64 << 40;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        let arrays = env.swap_arrays(&m, 0, false);
        let args = env.swap_args(false, mid);
        match predict(&env, &m, 0, &args, &arrays) {
            Ok(p) if p.out.ticks_crossed <= 10 => lo = mid,
            _ => hi = mid,
        }
    }
    let arrays = env.swap_arrays(&m, 0, false);
    let p = predict(&env, &m, 0, &env.swap_args(false, lo), &arrays).unwrap();
    assert_eq!(p.out.ticks_crossed, 10);
    let i = env.ix_swap(&m, &t, 0, env.swap_args(false, lo), &arrays);
    let cu10 = env
        .send(&[i], &[&t.kp])
        .expect("10 crossings within 200k CU");
    println!("10 crossings swap cu={cu10}");
    // Many crossings need a compute budget request; the result is still exact.
    for a_to_b in [true, false] {
        let arrays = env.swap_arrays(&m, 0, a_to_b);
        // The largest input that still succeeds crosses the most ticks.
        let (mut lo, mut hi) = (1u64, 1u64 << 50);
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            match predict(&env, &m, 0, &env.swap_args(a_to_b, mid), &arrays) {
                Ok(_) => lo = mid,
                Err(_) => hi = mid,
            }
        }
        let args = env.swap_args(a_to_b, lo);
        let c = checked_swap(&mut env, &m, &t, 0, args, &arrays).unwrap();
        println!(
            "swap a_to_b={a_to_b} crossed={} cu={}",
            c.out.ticks_crossed, c.cu
        );
        assert!(c.out.ticks_crossed >= 20);
        let per = (c.cu - cu) / u64::from(c.out.ticks_crossed);
        assert!(per < 12_000, "per crossing {per}");
    }
    let _ = Keypair::new();
}
