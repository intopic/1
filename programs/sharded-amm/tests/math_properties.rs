//! Exact property tests for the math kernel. Every check is an integer
//! inequality on 512-bit cross products, so it states the rational
//! definition directly instead of re-running the same formula.
//!
//! `AMM_STRESS_CASES` (default 20 000) and `AMM_STRESS_SEED` scale the run;
//! the final stress workflow runs them with millions of cases per job.

use sharded_amm::errors::AmmError;
use sharded_amm::math::fee_math::{fees_owed, growth_delta, split_fee};
use sharded_amm::math::liquidity_math::amounts_for_liquidity;
use sharded_amm::math::sqrt_price_math::{
    amount_a_delta, amount_b_delta, next_sqrt_price_from_a_input, next_sqrt_price_from_b_input,
};
use sharded_amm::math::swap_step::compute_swap_step;
use sharded_amm::math::tick_math::{
    sqrt_price_at_tick, tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64,
};
use sharded_amm::math::wide::{full_mul, U256, U512};

const MIN_TICK: i32 = -443_636;
const MAX_TICK: i32 = 443_636;
const FEES: [u32; 8] = [0, 1, 100, 500, 1000, 3000, 10_000, 100_000];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Random bit length first, so tiny and huge values are equally likely.
    fn wide_u128(&mut self, max_bits: u32) -> u128 {
        let bits = self.below(u64::from(max_bits) + 1) as u32;
        let v = u128::from(self.next()) | (u128::from(self.next()) << 64);
        if bits == 0 {
            0
        } else if bits >= 128 {
            v
        } else {
            v & ((1u128 << bits) - 1)
        }
    }
    fn wide_u64(&mut self) -> u64 {
        self.wide_u128(64) as u64
    }
    fn tick(&mut self) -> i32 {
        MIN_TICK + self.below((MAX_TICK - MIN_TICK + 1) as u64) as i32
    }
    /// Any price in the domain, not only tick prices.
    fn price(&mut self) -> u128 {
        let t = self.tick();
        let s = sqrt_price_at_tick(t).unwrap();
        if t == MAX_TICK {
            return s;
        }
        let next = sqrt_price_at_tick(t + 1).unwrap();
        s + (u128::from(self.next()) | (u128::from(self.next()) << 64)) % (next - s)
    }
}

fn cases() -> u64 {
    std::env::var("AMM_STRESS_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000)
}

fn rng(salt: u64) -> Rng {
    let seed: u64 = std::env::var("AMM_STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x2545_f491_4f6c_dd1d);
    Rng((seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1)
}

fn u(x: u128) -> U256 {
    U256::from(x)
}

fn w(x: U256) -> U512 {
    let l = x.0;
    U512([l[0], l[1], l[2], l[3], 0, 0, 0, 0])
}

#[test]
fn amount_deltas_are_exact_floor_and_ceil() {
    let mut r = rng(1);
    for _ in 0..cases() {
        let (s0, s1, l) = (r.price(), r.price(), r.wide_u128(128));
        let (sa, sb) = (s0.min(s1), s0.max(s1));
        let num_a = full_mul(u(l) << 64, u(sb - sa)); // L (sb - sa) 2^64
        let den_a = w(u(sa) * u(sb));
        let (fl, ce) = (
            amount_a_delta(s0, s1, l, false).unwrap(),
            amount_a_delta(s0, s1, l, true).unwrap(),
        );
        assert!(w(fl) * den_a <= num_a && num_a < (w(fl) + 1) * den_a);
        assert!(w(ce) * den_a >= num_a && (ce.is_zero() || (w(ce) - 1) * den_a < num_a));
        let num_b = full_mul(u(l), u(sb - sa));
        let q64 = U512::one() << 64;
        let (fl, ce) = (
            amount_b_delta(s0, s1, l, false).unwrap(),
            amount_b_delta(s0, s1, l, true).unwrap(),
        );
        assert!(w(fl) * q64 <= num_b && num_b < (w(fl) + 1) * q64);
        assert!(w(ce) * q64 >= num_b && (ce.is_zero() || (w(ce) - 1) * q64 < num_b));
    }
}

#[test]
fn next_price_never_overshoots() {
    let mut r = rng(2);
    for _ in 0..cases() {
        let (s, l, amt) = (r.price(), r.wide_u128(128).max(1), r.wide_u64());
        // A in: s' = ceil(L s 2^64 / (L 2^64 + amt s)); s' <= s.
        let na = next_sqrt_price_from_a_input(s, l, amt).unwrap();
        let num = full_mul(u(l) << 64, u(s));
        let den = w((u(l) << 64) + u(u128::from(amt)) * u(s));
        assert!(na <= s);
        assert!(w(u(na)) * den >= num && (w(u(na)) - 1) * den < num);
        // The pool never asks for more A than was offered to reach s'.
        if na < s {
            assert!(amount_a_delta(na, s, l, true).unwrap() <= u(u128::from(amt)));
        }
        // B in: s' = s + floor(amt 2^64 / L).
        match next_sqrt_price_from_b_input(s, l, amt) {
            Ok(nb) => {
                assert!(nb >= s);
                let step = U256::from(amt) << 64;
                assert_eq!(u(nb - s), step / u(l));
                assert!(amount_b_delta(s, nb, l, true).unwrap() <= u(u128::from(amt)));
            }
            Err(e) => assert_eq!(e, AmmError::MathOverflow),
        }
    }
}

#[test]
fn ticks_are_monotonic_and_invertible() {
    let mut r = rng(3);
    for _ in 0..cases() / 4 {
        let t = r.tick().clamp(MIN_TICK + 1, MAX_TICK - 1);
        let (lo, mid, hi) = (
            sqrt_price_at_tick(t - 1).unwrap(),
            sqrt_price_at_tick(t).unwrap(),
            sqrt_price_at_tick(t + 1).unwrap(),
        );
        assert!(lo < mid && mid < hi);
        assert_eq!(tick_at_sqrt_price(mid).unwrap(), t);
        assert_eq!(tick_at_sqrt_price(hi - 1).unwrap(), t);
        assert_eq!(tick_at_sqrt_price(mid - 1).unwrap(), t - 1);
        // Neighbouring ticks differ by about 0.005% in sqrt price.
        assert!((hi - mid) * 10_000 <= mid && (hi - mid) * 40_000 >= mid);
    }
}

/// The core solvency property of one step: the pool receives at least the
/// exact input for the price move and pays at most the exact output.
#[test]
fn swap_steps_favor_the_pool() {
    let mut r = rng(4);
    let mut checked = 0u64;
    for _ in 0..cases() {
        let s = r.price();
        let target = if r.below(3) == 0 {
            r.price()
        } else {
            let t = tick_at_sqrt_price(s).unwrap() + r.below(4001) as i32 - 2000;
            sqrt_price_at_tick(t.clamp(MIN_TICK, MAX_TICK)).unwrap()
        };
        let l = r.wide_u128(128);
        let gross = r.wide_u64();
        let fee = FEES[r.below(FEES.len() as u64) as usize];
        let step = match compute_swap_step(s, target, l, gross, fee) {
            Ok(step) => step,
            Err(AmmError::ZeroLiquidityGap) => {
                assert!(l == 0 && s != target && gross > 0);
                continue;
            }
            Err(AmmError::MathOverflow) => continue, // output above u64::MAX
            Err(e) => panic!("{e:?} for s={s} target={target} l={l} gross={gross} fee={fee}"),
        };
        let nxt = step.sqrt_price_next;
        assert!(nxt >= s.min(target) && nxt <= s.max(target));
        assert!((MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64).contains(&nxt));
        assert!(step.gross_consumed() <= gross);
        if step.reached_target {
            assert_eq!(nxt, target);
        } else {
            assert_eq!(step.gross_consumed(), gross);
        }
        // Fee is at least the nominal rate on the net input.
        let d = 1_000_000u128;
        assert!(
            u128::from(step.fee_nominal) * (d - u128::from(fee))
                >= u128::from(step.input_net) * u128::from(fee)
        );
        let (inp, out) = (
            w(u(u128::from(step.input_net))),
            w(u(u128::from(step.output))),
        );
        let q64 = U512::one() << 64;
        if target < s {
            // A in, B out.
            assert!(inp * w(u(nxt)) * w(u(s)) >= full_mul(u(l) << 64, u(s - nxt)));
            assert!(out * q64 <= full_mul(u(l), u(s - nxt)));
        } else if target > s {
            assert!(inp * q64 >= full_mul(u(l), u(nxt - s)));
            assert!(out * w(u(s)) * w(u(nxt)) <= full_mul(u(l) << 64, u(nxt - s)));
        }
        checked += 1;
    }
    assert!(checked > cases() / 2, "too few successful steps: {checked}");
}

/// Splitting one swap into many smaller ones never yields more output.
#[test]
fn splitting_input_does_not_pay_more() {
    let mut r = rng(5);
    for _ in 0..cases() / 20 {
        let s = sqrt_price_at_tick(r.tick().clamp(-200_000, 200_000)).unwrap();
        let a_to_b = r.below(2) == 0;
        let target = if a_to_b {
            MIN_SQRT_PRICE_X64
        } else {
            MAX_SQRT_PRICE_X64
        };
        let l = (r.wide_u128(100) | 1) << 20;
        let total = r.wide_u64() >> 2;
        let fee = FEES[r.below(FEES.len() as u64) as usize];
        let Ok(one) = compute_swap_step(s, target, l, total, fee) else {
            continue;
        };
        let parts = 2 + r.below(10);
        let (mut price, mut left, mut out) = (s, total, 0u128);
        for i in 0..parts {
            let chunk = if i + 1 == parts {
                left
            } else {
                left / (parts - i)
            };
            let Ok(step) = compute_swap_step(price, target, l, chunk, fee) else {
                break;
            };
            price = step.sqrt_price_next;
            left -= step.gross_consumed();
            out += u128::from(step.output);
        }
        if left == 0 {
            assert!(
                out <= u128::from(one.output) + parts as u128,
                "split paid {out} vs {}",
                one.output
            );
        }
    }
}

/// Fees distributed to positions never exceed the LP fee that was charged.
#[test]
fn fee_growth_never_over_distributes() {
    let mut r = rng(6);
    for _ in 0..cases() / 4 {
        let n = 1 + r.below(8) as usize;
        let ls: Vec<u128> = (0..n).map(|_| r.wide_u128(100)).collect();
        let total: u128 = ls.iter().sum();
        if total == 0 {
            continue;
        }
        let mut growth = U256::zero();
        let mut charged = 0u128;
        for _ in 0..1 + r.below(20) {
            let fee = r.wide_u64() >> 8;
            let (protocol, lp) = split_fee(fee, (r.below(5001)) as u16).unwrap();
            assert_eq!(protocol + lp, fee);
            growth += growth_delta(lp, total).unwrap();
            charged += u128::from(lp);
        }
        let paid: u128 = ls
            .iter()
            .map(|&l| u128::from(fees_owed(l, growth, U256::zero()).unwrap()))
            .sum();
        assert!(paid <= charged, "paid {paid} > charged {charged}");
        // Each position loses at most one raw unit per step to rounding.
        assert!(charged - paid <= 21 * n as u128 + 21);
    }
}

#[test]
fn liquidity_amounts_round_toward_the_pool() {
    let mut r = rng(7);
    for _ in 0..cases() / 4 {
        let mut ticks = [r.tick(), r.tick(), r.tick()];
        ticks.sort_unstable();
        if ticks[0] == ticks[2] {
            continue;
        }
        let (lower, upper) = (ticks[0], ticks[2]);
        let s = sqrt_price_at_tick(ticks[r.below(3) as usize]).unwrap();
        let (sa, sb) = (
            sqrt_price_at_tick(lower).unwrap(),
            sqrt_price_at_tick(upper).unwrap(),
        );
        let l = r.wide_u128(80);
        match (
            amounts_for_liquidity(s, sa, sb, l, true),
            amounts_for_liquidity(s, sa, sb, l, false),
        ) {
            (Ok(pay), Ok(back)) => assert!(
                back.0 <= pay.0 && back.1 <= pay.1 && pay.0 - back.0 <= 1 && pay.1 - back.1 <= 1
            ),
            (Err(AmmError::MathOverflow), _) | (_, Err(AmmError::MathOverflow)) => {}
            other => panic!("{other:?}"),
        }
    }
}
