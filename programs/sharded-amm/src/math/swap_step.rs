//! One exact-in swap step inside a single liquidity interval (research N05,
//! ADR-0001 points 6-6b).
//!
//! The fee rate `f` is charged on the gross input: `net = gross * (D - f) / D`.
//! When the step stops before the target, the whole remaining gross input is
//! consumed and split into `input_net + fee_nominal + rounding_dust`, so the
//! dust created by price quantization is reported instead of hidden.

use crate::constants::{FEE_RATE_DENOMINATOR, MAX_FEE_RATE_PPM};
use crate::errors::AmmError;

use super::sqrt_price_math::{
    amount_a_delta, amount_b_delta, next_sqrt_price_from_a_input, next_sqrt_price_from_b_input,
};
use super::wide::{to_u64, MathResult, U256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapStep {
    pub sqrt_price_next: u128,
    /// Input that moves the price, rounded up in favor of the pool.
    pub input_net: u64,
    /// `ceil(input_net * f / (D - f))`.
    pub fee_nominal: u64,
    /// Gross input left after `input_net + fee_nominal` in a final step.
    pub rounding_dust: u64,
    pub output: u64,
    pub reached_target: bool,
}

impl SwapStep {
    /// Fee split between LP and protocol; includes the rounding dust.
    pub fn total_fee(&self) -> u64 {
        self.fee_nominal + self.rounding_dust
    }

    pub fn gross_consumed(&self) -> u64 {
        self.input_net + self.total_fee()
    }
}

fn nominal_fee(input_net: u64, fee_ppm: u32) -> u64 {
    let d = u128::from(FEE_RATE_DENOMINATOR);
    let f = u128::from(fee_ppm);
    // < 2^64 * 2^17 / (D - f); fits u64 because fee <= input * f / (D - f) < input.
    let fee = (u128::from(input_net) * f).div_ceil(d - f);
    fee as u64
}

/// Moves from `sqrt_price` toward `sqrt_price_target` with `liquidity`,
/// spending at most `gross_remaining`. The direction is implied by the target:
/// below the current price means token A in (A -> B), above means token B in.
pub fn compute_swap_step(
    sqrt_price: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    gross_remaining: u64,
    fee_ppm: u32,
) -> MathResult<SwapStep> {
    if fee_ppm > MAX_FEE_RATE_PPM {
        return Err(AmmError::InvalidFeeRate);
    }
    if sqrt_price == sqrt_price_target || gross_remaining == 0 {
        // A crossing without consumption carries no fee.
        return Ok(SwapStep {
            sqrt_price_next: sqrt_price,
            reached_target: sqrt_price == sqrt_price_target,
            ..SwapStep::default()
        });
    }
    if liquidity == 0 {
        return Err(AmmError::ZeroLiquidityGap);
    }
    let a_to_b = sqrt_price_target < sqrt_price;
    let input_between = |from: u128, to: u128| -> MathResult<U256> {
        if a_to_b {
            amount_a_delta(to, from, liquidity, true)
        } else {
            amount_b_delta(from, to, liquidity, true)
        }
    };
    let d = u128::from(FEE_RATE_DENOMINATOR);
    let net_budget = (u128::from(gross_remaining) * (d - u128::from(fee_ppm)) / d) as u64;

    let to_target = input_between(sqrt_price, sqrt_price_target)?;
    let (sqrt_price_next, input_net, fee_nominal, rounding_dust, reached_target) =
        if to_target <= U256::from(net_budget) {
            let input_net = to_u64(to_target)?;
            let fee = nominal_fee(input_net, fee_ppm);
            // input_net <= floor(g (D - f) / D) implies input_net + fee <= g.
            if input_net
                .checked_add(fee)
                .is_none_or(|used| used > gross_remaining)
            {
                return Err(AmmError::MathInvariantViolated);
            }
            (sqrt_price_target, input_net, fee, 0, true)
        } else {
            let next = if a_to_b {
                next_sqrt_price_from_a_input(sqrt_price, liquidity, net_budget)?
            } else {
                next_sqrt_price_from_b_input(sqrt_price, liquidity, net_budget)?
            };
            // Rounding of `next` favors the pool, so it cannot pass the target
            // and the input it requires stays within the net budget.
            let past_target = if a_to_b {
                next < sqrt_price_target
            } else {
                next > sqrt_price_target
            };
            let input_net = to_u64(input_between(sqrt_price, next)?)?;
            if past_target || input_net > net_budget {
                return Err(AmmError::MathInvariantViolated);
            }
            let fee = nominal_fee(input_net, fee_ppm);
            let dust = gross_remaining
                .checked_sub(input_net)
                .and_then(|left| left.checked_sub(fee))
                .ok_or(AmmError::MathInvariantViolated)?;
            (next, input_net, fee, dust, next == sqrt_price_target)
        };
    let output = if a_to_b {
        amount_b_delta(sqrt_price_next, sqrt_price, liquidity, false)?
    } else {
        amount_a_delta(sqrt_price, sqrt_price_next, liquidity, false)?
    };
    Ok(SwapStep {
        sqrt_price_next,
        input_net,
        fee_nominal,
        rounding_dust,
        output: to_u64(output)?,
        reached_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::bitmap::tests::Rng;
    use crate::math::tick_math::{sqrt_price_at_tick, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};

    const ONE: u128 = 1 << 64;

    #[test]
    fn zero_liquidity_gap_is_an_error() {
        assert_eq!(
            compute_swap_step(ONE, ONE / 2, 0, 100, 3000),
            Err(AmmError::ZeroLiquidityGap)
        );
        // No distance to cover: allowed, no fee.
        let step = compute_swap_step(ONE, ONE, 0, 100, 3000).unwrap();
        assert!(step.reached_target && step.gross_consumed() == 0);
    }

    #[test]
    fn rejects_fee_above_bound() {
        assert_eq!(
            compute_swap_step(ONE, ONE / 2, 1, 1, MAX_FEE_RATE_PPM + 1),
            Err(AmmError::InvalidFeeRate)
        );
    }

    #[test]
    fn small_input_reports_dust_separately() {
        // Deep liquidity, 1 raw unit in at 0.30%: the price barely moves and
        // the unit cannot be split, so it is all fee (nominal or dust).
        let step = compute_swap_step(ONE, ONE / 2, 1 << 100, 1, 3000).unwrap();
        assert_eq!(step.gross_consumed(), 1);
        assert!(!step.reached_target);
        assert_eq!(step.output, 0);
    }

    #[test]
    fn randomized_step_invariants() {
        let mut rng = Rng(0x51ab_93e0_44c2_7f19);
        for _ in 0..4000 {
            let t0 = (rng.next() % 200_000) as i32 - 100_000;
            let t1 = t0 + (rng.next() % 20_000) as i32 - 10_000;
            let s = sqrt_price_at_tick(t0).unwrap();
            let target = sqrt_price_at_tick(t1).unwrap();
            let liquidity = u128::from(rng.next()) << (rng.next() % 50);
            let gross = rng.next() >> (rng.next() % 64);
            let fee = [0, 100, 500, 1000, 3000, 100_000][(rng.next() % 6) as usize];
            let Ok(step) = compute_swap_step(s, target, liquidity, gross, fee) else {
                continue; // output above u64::MAX; covered by the vector tests
            };
            assert!(step.gross_consumed() <= gross);
            if !step.reached_target {
                assert_eq!(
                    step.gross_consumed(),
                    gross,
                    "a final step consumes everything"
                );
            } else {
                assert_eq!(step.rounding_dust, 0);
            }
            let (lo, hi) = if target < s { (target, s) } else { (s, target) };
            assert!((lo..=hi).contains(&step.sqrt_price_next));
            assert!((MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64).contains(&step.sqrt_price_next));
            assert!(step.fee_nominal >= nominal_fee(step.input_net, fee));
        }
    }
}
