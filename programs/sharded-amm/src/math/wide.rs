//! Fixed-width integers for products that do not fit in `u128`.
//!
//! The token-A formulas need 288-bit intermediates (docs/numeric-spec.md
//! section 8), so every multiply-then-divide goes through a full 512-bit
//! product. Nothing here wraps silently: narrowing returns `MathOverflow`.

#![allow(clippy::all, clippy::pedantic)]

use crate::errors::AmmError;

pub type MathResult<T> = core::result::Result<T, AmmError>;

uint::construct_uint! {
    pub struct U256(4);
}

uint::construct_uint! {
    pub struct U512(8);
}

/// `2^64` and `2^128` as U256, the Q64 and Q128 scaling factors.
pub const Q64: U256 = U256([0, 1, 0, 0]);
pub const Q128: U256 = U256([0, 0, 1, 0]);

pub fn widen(x: U256) -> U512 {
    let w = x.0;
    U512([w[0], w[1], w[2], w[3], 0, 0, 0, 0])
}

pub fn narrow(x: U512) -> MathResult<U256> {
    let w = x.0;
    if w[4] | w[5] | w[6] | w[7] != 0 {
        return Err(AmmError::MathOverflow);
    }
    Ok(U256([w[0], w[1], w[2], w[3]]))
}

pub fn to_u128(x: U256) -> MathResult<u128> {
    if x.0[2] | x.0[3] != 0 {
        return Err(AmmError::MathOverflow);
    }
    Ok(x.low_u128())
}

pub fn to_u64(x: U256) -> MathResult<u64> {
    if x.0[1] | x.0[2] | x.0[3] != 0 {
        return Err(AmmError::MathOverflow);
    }
    Ok(x.low_u64())
}

/// Full 512-bit product of two U256 values; cannot overflow.
pub fn full_mul(a: U256, b: U256) -> U512 {
    widen(a) * widen(b)
}

/// `a * b / d` with an exact 512-bit intermediate, rounded down or up.
/// Fails if `d == 0` or the quotient does not fit in 256 bits.
pub fn mul_div(a: U256, b: U256, d: U256, round_up: bool) -> MathResult<U256> {
    if d.is_zero() {
        return Err(AmmError::DivisionByZero);
    }
    let (q, r) = full_mul(a, b).div_mod(widen(d));
    // q <= a * b < 2^512, so q + 1 cannot overflow U512.
    let q = if round_up && !r.is_zero() { q + 1 } else { q };
    narrow(q)
}

/// `n / d` rounded down or up.
pub fn div_round(n: U256, d: U256, round_up: bool) -> MathResult<U256> {
    if d.is_zero() {
        return Err(AmmError::DivisionByZero);
    }
    let (q, r) = n.div_mod(d);
    if round_up && !r.is_zero() {
        q.checked_add(U256::one()).ok_or(AmmError::MathOverflow)
    } else {
        Ok(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_needs_more_than_256_bits() {
        // (2^200) * (2^100) / 2^60 = 2^240: the product is 300 bits wide.
        let a = U256::one() << 200;
        let b = U256::one() << 100;
        let d = U256::one() << 60;
        assert_eq!(mul_div(a, b, d, false).unwrap(), U256::one() << 240);
        assert_eq!(mul_div(a, b, d, true).unwrap(), U256::one() << 240);
    }

    #[test]
    fn mul_div_rounding_and_errors() {
        let seven = U256::from(7u8);
        let two = U256::from(2u8);
        assert_eq!(
            mul_div(seven, U256::one(), two, false).unwrap(),
            U256::from(3u8)
        );
        assert_eq!(
            mul_div(seven, U256::one(), two, true).unwrap(),
            U256::from(4u8)
        );
        assert_eq!(
            mul_div(seven, seven, U256::zero(), false),
            Err(AmmError::DivisionByZero)
        );
        assert_eq!(
            mul_div(U256::MAX, U256::MAX, U256::one(), false),
            Err(AmmError::MathOverflow)
        );
        assert_eq!(div_round(U256::MAX, U256::one(), true).unwrap(), U256::MAX);
    }

    #[test]
    fn narrowing_is_checked() {
        assert_eq!(to_u64(U256::from(u64::MAX)).unwrap(), u64::MAX);
        assert_eq!(
            to_u64(U256::from(u64::MAX) + 1),
            Err(AmmError::MathOverflow)
        );
        assert_eq!(to_u128(U256::from(u128::MAX)).unwrap(), u128::MAX);
        assert_eq!(
            to_u128(U256::from(u128::MAX) + 1),
            Err(AmmError::MathOverflow)
        );
        assert_eq!(narrow(widen(U256::MAX)).unwrap(), U256::MAX);
        assert_eq!(narrow(widen(U256::MAX) + 1), Err(AmmError::MathOverflow));
        assert_eq!(Q64, U256::one() << 64);
        assert_eq!(Q128, U256::one() << 128);
    }
}
