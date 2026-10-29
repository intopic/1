//! Fee split and Q128 fee growth (research N07, ADR-0001 point 5).
//!
//! `global` and every `outside` value are U256 and checked: they never wrap.
//! `global` only grows; `outside` flips to `global - outside` on every
//! crossing, so it is not monotonic, but `0 <= outside <= global` always
//! holds and is enforced. `inside` is different. With the usual initialization
//! (`outside = global` when the tick is at or below the current tick) the
//! expression `global - below - above` can be negative for a range the price
//! has never been inside; see `inside_can_be_negative`. It is therefore
//! computed modulo 2^256 and only differences of it are used. A difference
//! `inside_now - checkpoint` equals the real growth inside the range since the
//! checkpoint, which is non-negative and below 2^256, so the modular
//! subtraction is exact.

use crate::constants::PROTOCOL_SHARE_DENOMINATOR;
use crate::errors::AmmError;

use super::wide::{mul_div, to_u64, MathResult, Q128, U256};

/// `(protocol, lp)` with `protocol = floor(total * share / 10_000)`.
pub fn split_fee(total_fee: u64, protocol_share_bps: u16) -> MathResult<(u64, u64)> {
    if protocol_share_bps > PROTOCOL_SHARE_DENOMINATOR {
        return Err(AmmError::InvalidProtocolShare);
    }
    let protocol = u128::from(total_fee) * u128::from(protocol_share_bps)
        / u128::from(PROTOCOL_SHARE_DENOMINATOR);
    let protocol = protocol as u64; // <= total_fee
    Ok((protocol, total_fee - protocol))
}

/// `floor(lp_fee * 2^128 / L)`. The remainder stays unallocated in the vault.
pub fn growth_delta(lp_fee: u64, liquidity: u128) -> MathResult<U256> {
    if liquidity == 0 {
        return Err(AmmError::ZeroLiquidity);
    }
    // lp_fee * 2^128 < 2^192 fits U256.
    Ok((U256::from(lp_fee) << 128) / U256::from(liquidity))
}

pub fn add_growth(global: U256, delta: U256) -> MathResult<U256> {
    global.checked_add(delta).ok_or(AmmError::MathOverflow)
}

/// Value of a tick's `outside` when it is first initialized.
pub fn initial_outside(tick: i32, tick_current: i32, global: U256) -> U256 {
    if tick <= tick_current {
        global
    } else {
        U256::zero()
    }
}

/// `outside` after the price crosses the tick: `global - outside`.
pub fn cross_outside(global: U256, outside: U256) -> MathResult<U256> {
    global
        .checked_sub(outside)
        .ok_or(AmmError::MathInvariantViolated)
}

/// Fee growth inside `[lower, upper)`, modulo 2^256 (see module docs).
pub fn growth_inside(
    tick_lower: i32,
    tick_upper: i32,
    tick_current: i32,
    global: U256,
    outside_lower: U256,
    outside_upper: U256,
) -> MathResult<U256> {
    if tick_lower >= tick_upper || outside_lower > global || outside_upper > global {
        return Err(AmmError::MathInvariantViolated);
    }
    let below = if tick_current >= tick_lower {
        outside_lower
    } else {
        global - outside_lower
    };
    let above = if tick_current < tick_upper {
        outside_upper
    } else {
        global - outside_upper
    };
    Ok(global.overflowing_sub(below).0.overflowing_sub(above).0)
}

/// Fees owed to a position with `liquidity` (the value before any change)
/// since its last checkpoint: `floor(L * (inside_now - checkpoint) / 2^128)`.
pub fn fees_owed(liquidity: u128, inside_now: U256, checkpoint: U256) -> MathResult<u64> {
    let delta = inside_now.overflowing_sub(checkpoint).0;
    to_u64(mul_div(U256::from(liquidity), delta, Q128, false)?)
}

/// Fees earned since the checkpoint and not yet credited to `owed`:
/// `floor(L * (inside_now - checkpoint) / 2^128) - credited`.
///
/// While a position's liquidity is unchanged its checkpoint stays put and
/// `credited` counts what was already moved to `owed`, so the rounding is
/// done once over the whole period: refreshing (collecting) after every swap
/// credits exactly what one refresh at the end would. A liquidity change
/// moves the checkpoint to `inside_now` and resets `credited` to 0; the
/// remainder lost there is below one token unit per change.
pub fn fees_since(
    liquidity: u128,
    inside_now: U256,
    checkpoint: U256,
    credited: u64,
) -> MathResult<u64> {
    fees_owed(liquidity, inside_now, checkpoint)?
        .checked_sub(credited)
        .ok_or(AmmError::MathInvariantViolated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(x: u64) -> U256 {
        U256::from(x)
    }

    #[test]
    fn split_matches_adr_0004_example() {
        // 0.30% of 1000 USDC (6 decimals) = 3 USDC; 20% protocol.
        assert_eq!(split_fee(3_000_000, 2000).unwrap(), (600_000, 2_400_000));
        assert_eq!(split_fee(1, 2000).unwrap(), (0, 1));
        assert_eq!(split_fee(7, 10_000).unwrap(), (7, 0));
        assert_eq!(split_fee(7, 10_001), Err(AmmError::InvalidProtocolShare));
    }

    /// Refreshes `n` times over `fees` (one swap each) on positions sharing
    /// the range; returns what each was credited.
    fn credited(ls: &[u128], fees: &[u64], every: bool) -> Vec<u64> {
        let total: u128 = ls.iter().sum();
        let mut global = U256::zero();
        let mut credited = vec![0u64; ls.len()];
        let mut paid = vec![0u64; ls.len()];
        for (k, f) in fees.iter().enumerate() {
            global = add_growth(global, growth_delta(*f, total).unwrap()).unwrap();
            if every || k + 1 == fees.len() {
                for (i, l) in ls.iter().enumerate() {
                    let e = fees_since(*l, global, U256::zero(), credited[i]).unwrap();
                    credited[i] += e;
                    paid[i] += e;
                }
            }
        }
        paid
    }

    #[test]
    fn frequent_refreshes_pay_what_one_refresh_pays() {
        // The re-audit's case: L = 3 and L = 1, four swaps of one unit each.
        // A ceil-advanced checkpoint paid [0, 1, 1, 0] = 2 to L = 3.
        assert_eq!(credited(&[3, 1], &[1; 4], true), vec![3, 1]);
        assert_eq!(credited(&[3, 1], &[1; 4], false), vec![3, 1]);
        // Random liquidities, fees and refresh patterns.
        let mut r = 0x5eed_u64;
        let mut next = || {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            r
        };
        for _ in 0..500 {
            let n = 1 + (next() % 4) as usize;
            let ls: Vec<u128> = (0..n).map(|_| 1 + u128::from(next() % 1_000)).collect();
            let fees: Vec<u64> = (0..1 + next() % 40).map(|_| next() % 50).collect();
            assert_eq!(credited(&ls, &fees, true), credited(&ls, &fees, false));
        }
        assert_eq!(
            fees_since(3, U256::zero(), U256::zero(), 1),
            Err(AmmError::MathInvariantViolated)
        );
    }

    #[test]
    fn growth_and_owed_round_down() {
        let l = 3u128;
        let delta = growth_delta(10, l).unwrap();
        // 10 / 3 per unit, three units -> 9 (one unit stays unallocated).
        assert_eq!(fees_owed(l, delta, U256::zero()).unwrap(), 9);
        assert_eq!(growth_delta(1, 0), Err(AmmError::ZeroLiquidity));
        assert_eq!(
            add_growth(U256::MAX, U256::one()),
            Err(AmmError::MathOverflow)
        );
    }

    /// Counterexample to "inside = global - below - above is never negative".
    #[test]
    fn inside_can_be_negative() {
        // Tick 100 is initialized while the price is below it (outside = 0).
        let mut outside_upper = initial_outside(100, 0, g(0));
        // The price rises through 100 when global = 10.
        outside_upper = cross_outside(g(10), outside_upper).unwrap();
        // At global = 20, price above both, tick 50 is initialized.
        let global = g(20);
        let outside_lower = initial_outside(50, 150, global);
        let below = outside_lower; // current >= lower
        let above = global - outside_upper; // current >= upper
        assert!(below + above > global, "real-valued inside is -10");
        let checkpoint = growth_inside(50, 100, 150, global, outside_lower, outside_upper).unwrap();
        // While the price stays above the range nothing is owed, even after growth.
        let later = growth_inside(50, 100, 150, g(35), outside_lower, outside_upper).unwrap();
        assert_eq!(fees_owed(1 << 64, later, checkpoint).unwrap(), 0);
        // The price drops into the range (crossing 100) and earns 5 more there.
        let outside_upper = cross_outside(g(35), outside_upper).unwrap();
        let inside = growth_inside(50, 100, 70, g(40), outside_lower, outside_upper).unwrap();
        let l = 1u128 << 64;
        let per_unit = inside.overflowing_sub(checkpoint).0;
        assert_eq!(per_unit, g(5));
        assert_eq!(fees_owed(l, inside, checkpoint).unwrap(), 0); // 5 * 2^64 / 2^128 rounds down
    }

    #[test]
    fn rejects_outside_above_global() {
        assert_eq!(
            cross_outside(g(1), g(2)),
            Err(AmmError::MathInvariantViolated)
        );
        assert_eq!(
            growth_inside(0, 10, 5, g(1), g(2), g(0)),
            Err(AmmError::MathInvariantViolated)
        );
        assert_eq!(
            growth_inside(10, 10, 5, g(1), g(0), g(0)),
            Err(AmmError::MathInvariantViolated)
        );
    }
}
