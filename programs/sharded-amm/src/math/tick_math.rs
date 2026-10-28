//! Tick <-> Q64.64 sqrt price (docs/numeric-spec.md section 2).
//!
//! `sqrt_price_at_tick(t)` targets `floor(sqrt(1.0001^t) * 2^64)`. It
//! multiplies normalized 128-bit mantissas of `sqrt(1.0001^(2^i))` from the
//! generated table, then shifts (t >= 0) or inverts `2^(64-e) / m` (t < 0). The tests check the result
//! against exact integer references for every committed vector.

use crate::constants::{MAX_TICK, MIN_TICK};
use crate::errors::AmmError;

use super::tick_table::{SQRT_POW, SQRT_POW_BITS};
use super::wide::{to_u128, MathResult, U256};

/// `sqrt_price_at_tick(MIN_TICK)`; equals the exact floor 4295048016.
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
/// `sqrt_price_at_tick(MAX_TICK)`; the exact floor (the ceil is ...062).
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_279_992_447_579_061;

const _: () = assert!((MAX_TICK as u32) < (1u32 << SQRT_POW_BITS));
const _: () = assert!(MIN_TICK == -MAX_TICK);

pub fn check_tick(tick: i32) -> MathResult<()> {
    if (MIN_TICK..=MAX_TICK).contains(&tick) {
        Ok(())
    } else {
        Err(AmmError::TickOutOfRange)
    }
}

pub fn check_sqrt_price(sqrt_price_x64: u128) -> MathResult<()> {
    if (MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64).contains(&sqrt_price_x64) {
        Ok(())
    } else {
        Err(AmmError::SqrtPriceOutOfRange)
    }
}

pub fn sqrt_price_at_tick(tick: i32) -> MathResult<u128> {
    check_tick(tick)?;
    let abs = tick.unsigned_abs();
    // R = sqrt(1.0001^|tick|) as a normalized binary float m * 2^e with
    // m in [2^127, 2^128). Only u128 arithmetic until the final step, which
    // keeps the SBF cost low (no U512).
    let mut m: u128 = 1 << 127;
    let mut e: i32 = -127;
    for (i, (pm, pe)) in SQRT_POW.iter().enumerate() {
        if abs & (1 << i) != 0 {
            let (hi, lo) = mul_u128(m, *pm);
            if hi >> 127 != 0 {
                m = hi;
                e += pe + 128;
            } else {
                m = (hi << 1) | (lo >> 127);
                e += pe + 127;
            }
        }
    }
    // R < 2^33, so e is in [-127, -95].
    if tick >= 0 {
        Ok(m >> (-(e + 64)) as u32)
    } else {
        // sqrt(P(-t)) * 2^64 = 2^(64 - e) / m.
        to_u128((U256::one() << (64 - e) as usize) / U256::from(m))
    }
}

/// Full 128 x 128 -> 256 bit product as (high, low) halves.
fn mul_u128(a: u128, b: u128) -> (u128, u128) {
    const LO: u128 = u64::MAX as u128;
    let (a1, a0) = (a >> 64, a & LO);
    let (b1, b0) = (b >> 64, b & LO);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & LO) + (p10 & LO);
    let lo = (p00 & LO) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// Largest tick `t` with `sqrt_price_at_tick(t) <= sqrt_price_x64`.
pub fn tick_at_sqrt_price(sqrt_price_x64: u128) -> MathResult<i32> {
    check_sqrt_price(sqrt_price_x64)?;
    // log2(s) is in [bits - 1, bits), so the tick is in
    // [(bits - 65) * K, (bits - 64) * K) with K = 2 / log2(1.0001) ~ 13863.6.
    let bits = (128 - sqrt_price_x64.leading_zeros()) as i32;
    let bound = |e: i32| [e * 13_863, e * 13_864];
    let mut lo = (bound(bits - 65).into_iter().min().unwrap_or(MIN_TICK) - 1).max(MIN_TICK);
    let mut hi = (bound(bits - 64).into_iter().max().unwrap_or(MAX_TICK) + 1).min(MAX_TICK);
    // The estimate is only a speed-up; fall back to the full domain if it is off.
    if sqrt_price_at_tick(lo)? > sqrt_price_x64 {
        lo = MIN_TICK;
    }
    if hi < MAX_TICK && sqrt_price_at_tick(hi)? <= sqrt_price_x64 {
        hi = MAX_TICK;
    }
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if sqrt_price_at_tick(mid)? <= sqrt_price_x64 {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Ok(lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::bitmap::tests::Rng;

    #[test]
    fn bounds_and_domain() {
        assert_eq!(sqrt_price_at_tick(MIN_TICK).unwrap(), MIN_SQRT_PRICE_X64);
        assert_eq!(sqrt_price_at_tick(MAX_TICK).unwrap(), MAX_SQRT_PRICE_X64);
        assert_eq!(sqrt_price_at_tick(0).unwrap(), 1u128 << 64);
        assert_eq!(
            sqrt_price_at_tick(MIN_TICK - 1),
            Err(AmmError::TickOutOfRange)
        );
        assert_eq!(
            sqrt_price_at_tick(MAX_TICK + 1),
            Err(AmmError::TickOutOfRange)
        );
        assert_eq!(tick_at_sqrt_price(MIN_SQRT_PRICE_X64).unwrap(), MIN_TICK);
        assert_eq!(tick_at_sqrt_price(MAX_SQRT_PRICE_X64).unwrap(), MAX_TICK);
        assert_eq!(
            tick_at_sqrt_price(MIN_SQRT_PRICE_X64 - 1),
            Err(AmmError::SqrtPriceOutOfRange)
        );
        assert_eq!(
            tick_at_sqrt_price(MAX_SQRT_PRICE_X64 + 1),
            Err(AmmError::SqrtPriceOutOfRange)
        );
    }

    #[test]
    fn strictly_monotonic_near_bounds_and_zero() {
        for range in [
            MIN_TICK..MIN_TICK + 2000,
            -2000..2000,
            MAX_TICK - 2000..MAX_TICK,
        ] {
            let mut prev = sqrt_price_at_tick(range.start).unwrap();
            for t in range.start + 1..range.end {
                let s = sqrt_price_at_tick(t).unwrap();
                assert!(s > prev, "tick {t}");
                prev = s;
            }
        }
    }

    #[test]
    fn tick_at_sqrt_price_is_the_inverse() {
        let mut rng = Rng(0x7ac3_11d5_09e4_2b61);
        for _ in 0..3000 {
            let span = (MAX_TICK - MIN_TICK) as u64;
            let t = MIN_TICK + 1 + (rng.next() % (span - 1)) as i32;
            let s = sqrt_price_at_tick(t).unwrap();
            assert_eq!(tick_at_sqrt_price(s).unwrap(), t);
            // One below the tick's price belongs to the previous tick.
            assert_eq!(tick_at_sqrt_price(s - 1).unwrap(), t - 1);
            let next = sqrt_price_at_tick(t + 1).unwrap();
            assert_eq!(tick_at_sqrt_price(next - 1).unwrap(), t);
        }
    }
}
