//! Liquidity bookkeeping and the token amounts a position represents
//! (docs/numeric-spec.md section 4).

use crate::constants::{MAX_TICK, MAX_TICK_SPACING};
use crate::errors::AmmError;

use super::sqrt_price_math::{amount_a_delta, amount_b_delta};
use super::wide::{to_u64, MathResult};

pub fn add_delta(liquidity: u128, delta: i128) -> MathResult<u128> {
    if delta >= 0 {
        liquidity.checked_add(delta.unsigned_abs())
    } else {
        liquidity.checked_sub(delta.unsigned_abs())
    }
    .ok_or(AmmError::MathOverflow)
}

/// `u128::MAX / N`, N = usable ticks for the spacing, so the sum of all
/// per-tick liquidity can never exceed `u128::MAX`.
pub fn max_liquidity_per_tick(tick_spacing: u16) -> MathResult<u128> {
    if tick_spacing == 0 || tick_spacing > MAX_TICK_SPACING {
        return Err(AmmError::InvalidTickSpacing);
    }
    let usable = (MAX_TICK as u128 / u128::from(tick_spacing)) * 2 + 1;
    Ok(u128::MAX / usable)
}

/// Token amounts for `liquidity` in `[sa, sb)` at the current price `s`.
/// `round_up` is true when the user pays (add), false when paid out (remove).
pub fn amounts_for_liquidity(
    s: u128,
    sa: u128,
    sb: u128,
    liquidity: u128,
    round_up: bool,
) -> MathResult<(u64, u64)> {
    if sa >= sb {
        return Err(AmmError::MathInvariantViolated);
    }
    let (a, b) = if s <= sa {
        (amount_a_delta(sa, sb, liquidity, round_up)?, 0.into())
    } else if s < sb {
        (
            amount_a_delta(s, sb, liquidity, round_up)?,
            amount_b_delta(sa, s, liquidity, round_up)?,
        )
    } else {
        (0.into(), amount_b_delta(sa, sb, liquidity, round_up)?)
    };
    Ok((to_u64(a)?, to_u64(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::bitmap::tests::Rng;
    use crate::math::tick_math::sqrt_price_at_tick;

    #[test]
    fn delta_is_checked() {
        assert_eq!(add_delta(5, -5).unwrap(), 0);
        assert_eq!(add_delta(5, -6), Err(AmmError::MathOverflow));
        assert_eq!(add_delta(u128::MAX, 1), Err(AmmError::MathOverflow));
        assert_eq!(add_delta(0, i128::MIN), Err(AmmError::MathOverflow));
    }

    #[test]
    fn per_tick_cap_covers_every_usable_tick() {
        for spacing in [1u16, 10, 60, MAX_TICK_SPACING] {
            let cap = max_liquidity_per_tick(spacing).unwrap();
            let usable = (MAX_TICK as u128 / u128::from(spacing)) * 2 + 1;
            assert!(cap.checked_mul(usable).is_some());
        }
        assert_eq!(max_liquidity_per_tick(0), Err(AmmError::InvalidTickSpacing));
    }

    /// Add then remove at the same price never returns more than was paid.
    #[test]
    fn add_remove_round_trip_has_no_profit() {
        let mut rng = Rng(0x0dd5_e11a_93b7_c204);
        for _ in 0..3000 {
            let lower = (rng.next() % 100_000) as i32 - 50_000;
            let upper = lower + 1 + (rng.next() % 5_000) as i32;
            let current = lower - 1_000 + (rng.next() % 7_000) as i32;
            let (s, sa, sb) = (
                sqrt_price_at_tick(current).unwrap(),
                sqrt_price_at_tick(lower).unwrap(),
                sqrt_price_at_tick(upper).unwrap(),
            );
            let l = u128::from(rng.next() >> (rng.next() % 64));
            let paid = amounts_for_liquidity(s, sa, sb, l, true).unwrap();
            let back = amounts_for_liquidity(s, sa, sb, l, false).unwrap();
            assert!(back.0 <= paid.0 && back.1 <= paid.1);
        }
    }
}
