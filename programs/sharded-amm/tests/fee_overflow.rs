//! D-01 regression (counterexample CE-01 of the mainnet analysis).
//!
//! A position whose liquidity never changes keeps one fee checkpoint, so the
//! fees earned since it (`floor(L * growth / 2^128)`) only grow. They passed
//! `u64::MAX` raw units after a few weeks of volume on a cheap 9-decimal
//! token, and then every refresh, collect and liquidity removal failed with
//! `MathOverflow`: the LP's principal was stuck. These tests drive the real
//! engine far past 2^64 units of fees and require that collecting and
//! exiting always work and pay exactly the LP fees, less rounding dust.
//!
//! `AMM_STRESS_CASES` and `AMM_STRESS_SEED` scale and seed the random test.

use anchor_lang::prelude::Pubkey;
use sharded_amm::engine::{modify_position, swap, Pool, PositionState, SwapParams};
use sharded_amm::math::sqrt_price_math::{amount_a_delta, amount_b_delta};
use sharded_amm::math::tick_math::sqrt_price_at_tick;
use sharded_amm::math::wide::to_u128;
use sharded_amm::state::{array_span, array_start_for, TickArray};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct Case {
    spacing: u16,
    fee: u32,
    share: u16,
    liquidity: u128,
    /// Refresh (collect) the position after every `collect_every` swaps.
    collect_every: u64,
    /// Fees each token must reach before everyone exits.
    target: u128,
}

#[derive(Debug, Default)]
struct Totals {
    swaps: u64,
    collects: u64,
    lp_a: u128,
    lp_b: u128,
    paid_a: u128,
    paid_b: u128,
}

fn refs(tick: i32, spacing: u16, arrays: &mut [TickArray], a_to_b: bool) -> Vec<&mut TickArray> {
    let start = array_start_for(tick, spacing);
    let idx = arrays
        .iter()
        .position(|a| a.start_tick_index == start)
        .unwrap();
    if a_to_b {
        arrays[..=idx].iter_mut().rev().take(3).collect()
    } else {
        arrays[idx..].iter_mut().take(3).collect()
    }
}

fn run(c: &Case, r: &mut Rng) -> Totals {
    let s = i32::from(c.spacing);
    let (lower, upper, swing) = (-64 * s, 64 * s, 24 * s);
    let span = array_span(c.spacing);
    let first = array_start_for(lower, c.spacing) - span;
    let mut arrays: Vec<TickArray> = (0..7)
        .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
        .collect();
    let mut pool = Pool::at_price(sqrt_price_at_tick(0).unwrap()).unwrap();
    let mut pos = PositionState {
        lower,
        upper,
        ..Default::default()
    };
    // The whole range in either token must fit a SPL balance (else no vault
    // could hold it): halve L until it does.
    let (sa, sb) = (sqrt_price_at_tick(lower).unwrap(), sqrt_price_at_tick(upper).unwrap());
    let fits = |l: u128| {
        to_u128(amount_a_delta(sa, sb, l, true).unwrap()).unwrap() <= u128::from(u64::MAX)
            && to_u128(amount_b_delta(sa, sb, l, true).unwrap()).unwrap() <= u128::from(u64::MAX)
    };
    let mut liquidity = c.liquidity;
    while !fits(liquidity) {
        liquidity /= 2;
    }
    {
        let mut r: Vec<&mut TickArray> = arrays.iter_mut().collect();
        modify_position(&mut pool, &mut r, c.spacing, &mut pos, liquidity as i128).unwrap();
    }
    let mut t = Totals::default();
    let d = 1_000_000u128;
    while t.lp_a < c.target || t.lp_b < c.target {
        let down = pool.tick >= 0;
        let target = sqrt_price_at_tick(if down { -swing } else { swing }).unwrap();
        let net = if down {
            amount_a_delta(target, pool.sqrt_price, liquidity, true).unwrap()
        } else {
            amount_b_delta(pool.sqrt_price, target, liquidity, true).unwrap()
        };
        let net = to_u128(net).unwrap();
        let gross = (net * d).div_ceil(d - u128::from(c.fee));
        let cap = 1 + u128::from(r.next() >> 1);
        let amount = gross.min(cap) as u64;
        let o = {
            let mut a = refs(pool.tick, c.spacing, &mut arrays, down);
            swap(
                &mut pool,
                &mut a,
                SwapParams {
                    a_to_b: down,
                    amount_in: amount,
                    sqrt_price_limit: target,
                    fee_rate_ppm: c.fee,
                    protocol_share_bps: c.share,
                    tick_spacing: c.spacing,
                },
            )
            .unwrap()
        };
        let lp = u128::from(o.total_fee - o.protocol_fee);
        if down {
            t.lp_a += lp;
        } else {
            t.lp_b += lp;
        }
        t.swaps += 1;
        if t.swaps % c.collect_every == 0 {
            let mut a: Vec<&mut TickArray> = arrays.iter_mut().collect();
            modify_position(&mut pool, &mut a, c.spacing, &mut pos, 0)
                .unwrap_or_else(|e| panic!("collect after {t:?} failed: {e:?}"));
            t.paid_a += u128::from(pos.owed_a);
            t.paid_b += u128::from(pos.owed_b);
            (pos.owed_a, pos.owed_b) = (0, 0);
            // The protocol collects too: its unpaid fees sit in the u64 vault.
            (pool.protocol_a, pool.protocol_b) = (0, 0);
            t.collects += 1;
        }
        assert!(t.swaps < 2_000_000, "no progress: {t:?}");
    }
    // Remove everything, then collect what is left.
    let mut a: Vec<&mut TickArray> = arrays.iter_mut().collect();
    modify_position(&mut pool, &mut a, c.spacing, &mut pos, -(liquidity as i128))
        .unwrap_or_else(|e| panic!("exit after {t:?} failed: {e:?}"));
    modify_position(&mut pool, &mut a, c.spacing, &mut pos, 0).unwrap();
    t.paid_a += u128::from(pos.owed_a);
    t.paid_b += u128::from(pos.owed_b);
    assert_eq!(pool.liquidity, 0);
    assert_eq!(pos.liquidity, 0);
    // Never more than the LPs' share; less only by rounding: below one unit
    // per swap (growth rounds down) and per period (owed rounds down).
    let slack = u128::from(t.swaps + t.collects + 2);
    for (paid, lp, token) in [(t.paid_a, t.lp_a, "A"), (t.paid_b, t.lp_b, "B")] {
        assert!(paid <= lp, "{token}: paid {paid} > LP fees {lp}");
        assert!(lp - paid <= slack, "{token}: lost {} > {slack}", lp - paid);
    }
    t
}

/// CE-01 exactly: L = 2^66, spacing 60, fee 1%, protocol 16%, collect every
/// 10 swaps. The old code failed once credited fees reached u64::MAX.
#[test]
fn fees_past_u64_can_be_collected_and_principal_exits() {
    let c = Case {
        spacing: 60,
        fee: 10_000,
        share: 1600,
        liquidity: 1 << 66,
        collect_every: 10,
        target: 4 * u128::from(u64::MAX),
    };
    let t = run(&c, &mut Rng(0x5eed));
    assert!(t.paid_a > 3 * u128::from(u64::MAX), "{t:?}");
    assert!(t.paid_b > 3 * u128::from(u64::MAX), "{t:?}");
}

/// Random spacing, fee, protocol share, liquidity up to 2^100 (or the
/// largest that a u64 deposit allows) and collect pattern; every case passes 2^64 units of fees per token.
#[test]
fn random_long_fee_periods_never_block_exit() {
    let mut r = Rng(env("AMM_STRESS_SEED", 0xfee5) | 1);
    let cases = (env("AMM_STRESS_CASES", 20_000) / 1_000).max(4);
    for _ in 0..cases {
        let c = Case {
            spacing: [1u16, 10, 60, 200][r.below(4) as usize],
            fee: [3000, 10_000, 20_000][r.below(3) as usize],
            share: [0, 1600, 5000][r.below(3) as usize],
            liquidity: 1u128 << (76 + r.below(25)),
            collect_every: 1 + r.below(64),
            target: u128::from(u64::MAX) + u128::from(r.next()),
        };
        run(&c, &mut r);
    }
    eprintln!("STATS fee_overflow cases {cases}");
}
