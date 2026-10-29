//! Tick <-> Q64.64 sqrt price (docs/numeric-spec.md section 2).
//!
//! `sqrt_price_at_tick(t)` targets `floor(sqrt(1.0001^t) * 2^64)`. It
//! multiplies normalized 128-bit mantissas of `sqrt(1.0001^(+-2^i))` from the
//! generated tables, then shifts the result to Q64.64. The tests check the result
//! against exact integer references for every committed vector.

use crate::constants::{MAX_TICK, MIN_TICK};
use crate::errors::AmmError;

use super::tick_table::{SQRT_POW, SQRT_POW_BITS, SQRT_POW_NEG};
use super::wide::{mul_u128, MathResult};

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
    // R = sqrt(1.0001^tick) as a normalized binary float m * 2^e with
    // m in [2^127, 2^128), multiplied from the positive or negative powers.
    // Only u128 arithmetic, which keeps the SBF cost low.
    let table = if tick >= 0 { &SQRT_POW } else { &SQRT_POW_NEG };
    let mut m: u128 = 1 << 127;
    let mut e: i32 = -127;
    for (i, (pm, pe)) in table.iter().enumerate() {
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
    // 2^-33 < R < 2^33, so the shift -(e + 64) is in [31, 96].
    Ok(m >> (-(e + 64)) as u32)
}

/// `round(2^32 / log2(sqrt(1.0001)))`, the ticks per unit of `log2(s)`.
const TICKS_PER_LOG2_Q32: i128 = 59_543_866_431_248;
/// Fraction bits of the `log2` estimate; more bits only cost squarings.
const LOG2_FRACTION_BITS: u32 = 20;

/// Largest tick `t` with `sqrt_price_at_tick(t) <= sqrt_price_x64`.
///
/// A fixed-point `log2` estimate gives a tick within one of the answer; the
/// exact forward function then settles it, so the result is exact whatever
/// the estimate's error.
pub fn tick_at_sqrt_price(sqrt_price_x64: u128) -> MathResult<i32> {
    check_sqrt_price(sqrt_price_x64)?;
    let msb = 127 - sqrt_price_x64.leading_zeros();
    let mut r = sqrt_price_x64 << (127 - msb);
    let mut log2 = (i64::from(msb) - 64) << LOG2_FRACTION_BITS;
    for bit in (0..LOG2_FRACTION_BITS).rev() {
        // r in [2^127, 2^128) stands for r / 2^127 in [1, 2); square it.
        let (hi, lo) = mul_u128(r, r);
        if hi >> 127 != 0 {
            r = hi;
            log2 |= 1 << bit;
        } else {
            r = (hi << 1) | (lo >> 127);
        }
    }
    let estimate = (i128::from(log2) * TICKS_PER_LOG2_Q32) >> (32 + LOG2_FRACTION_BITS);
    let mut tick = (estimate as i32).clamp(MIN_TICK, MAX_TICK);
    while tick > MIN_TICK && sqrt_price_at_tick(tick)? > sqrt_price_x64 {
        tick -= 1;
    }
    while tick < MAX_TICK && sqrt_price_at_tick(tick + 1)? <= sqrt_price_x64 {
        tick += 1;
    }
    Ok(tick)
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

    #[test]
    fn inverse_on_a_dense_grid_and_estimate_is_tight() {
        // Every 5th tick over the whole domain; the log2 estimate must land
        // within one tick so the settle loops run at most twice.
        let mut t = MIN_TICK + 1;
        while t < MAX_TICK {
            let s = sqrt_price_at_tick(t).unwrap();
            assert_eq!(tick_at_sqrt_price(s).unwrap(), t);
            assert_eq!(tick_at_sqrt_price(s - 1).unwrap(), t - 1);
            let msb = 127 - s.leading_zeros();
            let mut r = s << (127 - msb);
            let mut log2 = (i64::from(msb) - 64) << LOG2_FRACTION_BITS;
            for bit in (0..LOG2_FRACTION_BITS).rev() {
                let (hi, lo) = mul_u128(r, r);
                if hi >> 127 != 0 {
                    r = hi;
                    log2 |= 1 << bit;
                } else {
                    r = (hi << 1) | (lo >> 127);
                }
            }
            let est = ((i128::from(log2) * TICKS_PER_LOG2_Q32) >> (32 + LOG2_FRACTION_BITS)) as i32;
            assert!((est - t).abs() <= 1, "tick {t} estimate {est}");
            t += 5;
        }
    }
}
