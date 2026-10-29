//! Usable depth of a shard before it becomes routable (rule 4 of the
//! product rules, 2026-10-29): high liquidity at the current price alone does
//! not prove the shard can trade both ways.
//!
//! Two probe swaps run on copies of the pool and its tick arrays, never on
//! the accounts: selling `probe_a` of A and, separately, selling `probe_b`
//! of B must each fill completely before the price moves `max_impact_ticks`
//! from the current tick (1 tick is ~1 bp of price). A probe that runs out
//! of liquidity, out of the given arrays or into that limit fails with
//! `InsufficientDepth`. The
//! caller passes copies; the probes change them.

use crate::constants::{MAX_TICK, MIN_TICK};
use crate::errors::AmmError;
use crate::math::tick_math::sqrt_price_at_tick;
use crate::state::TickArray;

use super::{swap, Pool, SwapParams};

#[derive(Clone, Copy, Debug)]
pub struct DepthProbe {
    pub probe_a: u64,
    pub probe_b: u64,
    pub max_impact_ticks: u16,
    pub fee_rate_ppm: u32,
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
}

/// Ticks after the A -> B and the B -> A probe.
pub fn probe_depth(
    pool: &Pool,
    arrays_a_to_b: &mut [TickArray],
    arrays_b_to_a: &mut [TickArray],
    p: DepthProbe,
) -> Result<(i32, i32), AmmError> {
    let impact = i32::from(p.max_impact_ticks);
    let one = |arrays: &mut [TickArray], a_to_b: bool, amount: u64| -> Result<i32, AmmError> {
        let limit_tick = if a_to_b {
            pool.tick.saturating_sub(impact).max(MIN_TICK + 1)
        } else {
            pool.tick.saturating_add(impact).min(MAX_TICK - 1)
        };
        let mut copy = *pool;
        let mut refs: Vec<&mut TickArray> = arrays.iter_mut().collect();
        swap(
            &mut copy,
            &mut refs,
            SwapParams {
                a_to_b,
                amount_in: amount,
                sqrt_price_limit: sqrt_price_at_tick(limit_tick)?,
                fee_rate_ppm: p.fee_rate_ppm,
                protocol_share_bps: p.protocol_share_bps,
                tick_spacing: p.tick_spacing,
            },
        )
        .map_err(|e| match e {
            AmmError::PriceLimitReached
            | AmmError::InvalidPriceLimit
            | AmmError::ZeroLiquidityGap
            | AmmError::TickArraysExhausted => AmmError::InsufficientDepth,
            e => e,
        })?;
        Ok(copy.tick)
    };
    Ok((
        one(arrays_a_to_b, true, p.probe_a)?,
        one(arrays_b_to_a, false, p.probe_b)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{modify_position, PositionState};
    use crate::state::{array_span, array_start_for};
    use anchor_lang::prelude::Pubkey;

    const SPACING: u16 = 60;

    /// One position over `[lower, upper)` with `liquidity`, price at `tick`.
    fn shard(tick: i32, lower: i32, upper: i32, liquidity: u128) -> (Pool, Vec<TickArray>) {
        let span = array_span(SPACING);
        let first = array_start_for(-12_000, SPACING);
        let mut arrays: Vec<TickArray> = (0..13)
            .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
            .collect();
        let mut pool = Pool::at_price(sqrt_price_at_tick(tick).unwrap()).unwrap();
        let mut pos = PositionState {
            lower,
            upper,
            liquidity: 0,
            inside_last_a: Default::default(),
            inside_last_b: Default::default(),
            owed_a: 0,
            owed_b: 0,
        };
        let mut refs: Vec<&mut TickArray> = arrays.iter_mut().collect();
        modify_position(&mut pool, &mut refs, SPACING, &mut pos, liquidity as i128).unwrap();
        (pool, arrays)
    }

    /// Copies of up to three arrays from the current tick each way.
    fn legs(pool: &Pool, arrays: &[TickArray]) -> (Vec<TickArray>, Vec<TickArray>) {
        let i = arrays
            .iter()
            .position(|a| a.start_tick_index == array_start_for(pool.tick, SPACING))
            .unwrap();
        let down = arrays[i.saturating_sub(2)..=i]
            .iter()
            .rev()
            .copied()
            .collect();
        let up = arrays[i..(i + 3).min(arrays.len())].to_vec();
        (down, up)
    }

    fn probe(amount: u64, impact: u16) -> DepthProbe {
        DepthProbe {
            probe_a: amount,
            probe_b: amount,
            max_impact_ticks: impact,
            fee_rate_ppm: 3_000,
            protocol_share_bps: 2_000,
            tick_spacing: SPACING,
        }
    }

    fn run(pool: &Pool, arrays: &[TickArray], p: DepthProbe) -> Result<(i32, i32), AmmError> {
        let (mut down, mut up) = legs(pool, arrays);
        probe_depth(pool, &mut down, &mut up, p)
    }

    #[test]
    fn deep_shard_passes_within_the_limit() {
        let (pool, arrays) = shard(0, -6_000, 6_000, 1_000_000_000_000);
        let (a, b) = run(&pool, &arrays, probe(1_000_000_000, 100)).unwrap();
        assert!(a <= pool.tick && a >= pool.tick - 100, "a {a}");
        assert!(b >= pool.tick && b <= pool.tick + 100, "b {b}");
        // The pool itself is untouched; the probes ran on a copy.
        assert_eq!(pool, shard(0, -6_000, 6_000, 1_000_000_000_000).0);
    }

    #[test]
    fn shallow_shard_fails() {
        let (pool, arrays) = shard(0, -6_000, 6_000, 1_000_000);
        assert_eq!(
            run(&pool, &arrays, probe(1_000_000_000, 100)),
            Err(AmmError::InsufficientDepth)
        );
    }

    #[test]
    fn high_liquidity_on_one_side_only_fails() {
        // Large L, but the price sits 60 ticks below the top of the only
        // position: selling B for A runs out within 1%.
        let (pool, arrays) = shard(5_940, -6_000, 6_000, 1_000_000_000_000_000);
        assert!(pool.liquidity > 0);
        assert_eq!(
            run(&pool, &arrays, probe(1_000_000_000_000_000, 100)),
            Err(AmmError::InsufficientDepth)
        );
        // A small B probe and a normal A probe pass on the same shard.
        let (mut down, _) = legs(&pool, &arrays);
        let (_, mut up) = legs(&pool, &arrays);
        let ok = probe_depth(
            &pool,
            &mut down,
            &mut up,
            DepthProbe {
                probe_b: 1_000_000,
                ..probe(1_000_000_000, 100)
            },
        );
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn missing_arrays_fail() {
        let (pool, arrays) = shard(0, -6_000, 6_000, 1_000_000_000_000);
        let (mut down, _) = legs(&pool, &arrays);
        // No arrays is an account error, not a depth result.
        assert_eq!(
            probe_depth(&pool, &mut down, &mut [], probe(1_000, 100)),
            Err(AmmError::TickArrayMismatch)
        );
    }
}
