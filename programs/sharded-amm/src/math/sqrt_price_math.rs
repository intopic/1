//! Token amounts between two sqrt prices and the next price for an input
//! (docs/numeric-spec.md section 3). Every rounding favors the pool: inputs
//! the user owes round up, outputs paid out round down, and the next price
//! moves less than the exact value would.

use crate::errors::AmmError;

use super::wide::{div_round, mul_div, to_u128, MathResult, Q64, U256};

fn ordered(s0: u128, s1: u128) -> MathResult<(U256, U256)> {
    let (lo, hi) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
    if lo == 0 {
        return Err(AmmError::SqrtPriceOutOfRange);
    }
    Ok((U256::from(lo), U256::from(hi)))
}

/// `L * (sb - sa) * 2^64 / (sa * sb)`: token A between the two prices.
pub fn amount_a_delta(s0: u128, s1: u128, liquidity: u128, round_up: bool) -> MathResult<U256> {
    let (sa, sb) = ordered(s0, s1)?;
    // sa * sb < 2^192; the numerator needs up to 288 bits.
    mul_div(U256::from(liquidity) << 64, sb - sa, sa * sb, round_up)
}

/// `L * (sb - sa) / 2^64`: token B between the two prices.
pub fn amount_b_delta(s0: u128, s1: u128, liquidity: u128, round_up: bool) -> MathResult<U256> {
    let (sa, sb) = ordered(s0, s1)?;
    mul_div(U256::from(liquidity), sb - sa, Q64, round_up)
}

/// Price after adding `amount_a` of token A (price moves down):
/// `ceil(L * s * 2^64 / (L * 2^64 + amount_a * s))`.
pub fn next_sqrt_price_from_a_input(s: u128, liquidity: u128, amount_a: u64) -> MathResult<u128> {
    if liquidity == 0 {
        return Err(AmmError::ZeroLiquidity);
    }
    if amount_a == 0 {
        return Ok(s);
    }
    let l_q64 = U256::from(liquidity) << 64;
    let s = U256::from(s);
    // L * 2^64 < 2^192 and amount * s < 2^160, so the sum fits in U256.
    let denominator = l_q64 + U256::from(amount_a) * s;
    to_u128(mul_div(l_q64, s, denominator, true)?)
}

/// Price after adding `amount_b` of token B (price moves up):
/// `s + floor(amount_b * 2^64 / L)`.
pub fn next_sqrt_price_from_b_input(s: u128, liquidity: u128, amount_b: u64) -> MathResult<u128> {
    if liquidity == 0 {
        return Err(AmmError::ZeroLiquidity);
    }
    let step = div_round(U256::from(amount_b) << 64, U256::from(liquidity), false)?;
    s.checked_add(to_u128(step)?).ok_or(AmmError::MathOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::tick_math::{MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};

    const ONE: u128 = 1 << 64;

    #[test]
    fn unit_price_examples() {
        // L = 2^64 over [1, 2] in price units of sqrt: A = L * (1 - 1/2) = 2^63.
        let l = 1u128 << 64;
        assert_eq!(
            amount_a_delta(ONE, 2 * ONE, l, false).unwrap(),
            U256::from(1u128 << 63)
        );
        assert_eq!(
            amount_b_delta(ONE, 2 * ONE, l, false).unwrap(),
            U256::from(1u128 << 64)
        );
        // Order of the arguments does not matter.
        assert_eq!(
            amount_a_delta(2 * ONE, ONE, l, true).unwrap(),
            amount_a_delta(ONE, 2 * ONE, l, true).unwrap()
        );
    }

    #[test]
    fn rounding_direction() {
        let (s0, s1, l) = (ONE + 12_345, 3 * ONE + 7, 1_000_003u128);
        for f in [amount_a_delta, amount_b_delta] {
            let down = f(s0, s1, l, false).unwrap();
            let up = f(s0, s1, l, true).unwrap();
            assert!(up == down || up == down + 1);
        }
        // Next price never overshoots: A input moves the price down less.
        let next = next_sqrt_price_from_a_input(2 * ONE, 1 << 70, 1 << 40).unwrap();
        assert!(next < 2 * ONE);
        let paid = amount_a_delta(next, 2 * ONE, 1 << 70, true).unwrap();
        assert!(paid <= U256::from(1u64 << 40));
    }

    #[test]
    fn extremes_do_not_overflow() {
        let a = amount_a_delta(MIN_SQRT_PRICE_X64, MAX_SQRT_PRICE_X64, u128::MAX, true).unwrap();
        let b = amount_b_delta(MIN_SQRT_PRICE_X64, MAX_SQRT_PRICE_X64, u128::MAX, true).unwrap();
        assert!(a > U256::from(u64::MAX) && b > U256::from(u64::MAX));
        let s = next_sqrt_price_from_a_input(MAX_SQRT_PRICE_X64, u128::MAX, u64::MAX).unwrap();
        assert!(s < MAX_SQRT_PRICE_X64);
        assert_eq!(
            next_sqrt_price_from_b_input(ONE, 0, 1),
            Err(AmmError::ZeroLiquidity)
        );
        assert_eq!(
            next_sqrt_price_from_a_input(ONE, 0, 1),
            Err(AmmError::ZeroLiquidity)
        );
        assert_eq!(
            amount_a_delta(0, ONE, 1, true),
            Err(AmmError::SqrtPriceOutOfRange)
        );
    }
}
