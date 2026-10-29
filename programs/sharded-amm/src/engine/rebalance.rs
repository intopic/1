//! Capital-free rebalancing between two shards (ADR-0006).
//!
//! `lo` is the shard where A is cheaper. With `input_is_a == false` the
//! first leg buys A on `lo` with `amount_in` B and the second sells all of it
//! on `hi`; with `input_is_a == true` the first leg sells `amount_in` A on
//! `hi` and the second buys A back on `lo`. Each leg is an ordinary
//! `swap` at that shard's own fee, so each shard ends exactly as after two
//! normal swaps. The first leg stops at the other shard's price and the
//! second at the first shard's new price, so the prices converge and never
//! cross.
//!
//! On error the pools and arrays may be partly updated; the program reverts
//! the transaction and host callers must discard them.

use crate::errors::AmmError;
use crate::state::TickArray;

use super::{swap, Pool, SwapOutcome, SwapParams};

#[derive(Clone, Copy, Debug)]
pub struct RebalanceParams {
    pub input_is_a: bool,
    pub amount_in: u64,
    pub min_profit: u64,
    pub fee_lo_ppm: u32,
    pub fee_hi_ppm: u32,
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RebalanceOutcome {
    pub first: SwapOutcome,
    pub second: SwapOutcome,
    /// `second.amount_out - amount_in`, paid to the caller.
    pub profit: u64,
}

/// Runs both legs. `lo_arrays` and `hi_arrays` start at each shard's current
/// tick, in the direction of that shard's leg.
pub fn rebalance(
    lo: &mut Pool,
    lo_arrays: &mut [&mut TickArray],
    hi: &mut Pool,
    hi_arrays: &mut [&mut TickArray],
    p: RebalanceParams,
) -> Result<RebalanceOutcome, AmmError> {
    if lo.sqrt_price >= hi.sqrt_price {
        return Err(AmmError::RebalanceNoPriceGap);
    }
    let leg = |a_to_b: bool, amount_in: u64, limit: u128, fee: u32| SwapParams {
        a_to_b,
        amount_in,
        sqrt_price_limit: limit,
        fee_rate_ppm: fee,
        protocol_share_bps: p.protocol_share_bps,
        tick_spacing: p.tick_spacing,
    };
    let (first, second) = if p.input_is_a {
        let first = swap(
            hi,
            hi_arrays,
            leg(true, p.amount_in, lo.sqrt_price, p.fee_hi_ppm),
        )?;
        let second = swap(
            lo,
            lo_arrays,
            leg(false, first.amount_out, hi.sqrt_price, p.fee_lo_ppm),
        )?;
        (first, second)
    } else {
        let first = swap(
            lo,
            lo_arrays,
            leg(false, p.amount_in, hi.sqrt_price, p.fee_lo_ppm),
        )?;
        let second = swap(
            hi,
            hi_arrays,
            leg(true, first.amount_out, lo.sqrt_price, p.fee_hi_ppm),
        )?;
        (first, second)
    };
    let profit = second
        .amount_out
        .checked_sub(p.amount_in)
        .filter(|&g| g > 0 && g >= p.min_profit)
        .ok_or(AmmError::RebalanceUnprofitable)?;
    Ok(RebalanceOutcome {
        first,
        second,
        profit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{modify_position, PositionState};
    use crate::math::tick_math::sqrt_price_at_tick;
    use crate::state::{array_span, array_start_for};
    use anchor_lang::prelude::Pubkey;

    const SPACING: u16 = 60;

    struct Shard {
        pool: Pool,
        arrays: Vec<TickArray>,
    }

    impl Shard {
        fn at_tick(tick: i32, liquidity: u128) -> Self {
            let span = array_span(SPACING);
            let first = array_start_for(-6_000, SPACING);
            let arrays = (0..8)
                .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
                .collect();
            let mut s = Self {
                pool: Pool::at_price(sqrt_price_at_tick(tick).unwrap()).unwrap(),
                arrays,
            };
            let mut pos = PositionState {
                lower: -6_000,
                upper: 6_000,
                liquidity: 0,
                inside_last_a: Default::default(),
                inside_last_b: Default::default(),
                owed_a: 0,
                owed_b: 0,
                credited_a: 0,
                credited_b: 0,
            };
            let mut refs: Vec<&mut TickArray> = s.arrays.iter_mut().collect();
            modify_position(&mut s.pool, &mut refs, SPACING, &mut pos, liquidity as i128).unwrap();
            s
        }
    }

    /// Up to three arrays from the current tick in the swap direction.
    fn refs<'a>(pool: &Pool, arrays: &'a mut [TickArray], a_to_b: bool) -> Vec<&'a mut TickArray> {
        let start = array_start_for(pool.tick, SPACING);
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

    fn params(input_is_a: bool, amount_in: u64) -> RebalanceParams {
        RebalanceParams {
            input_is_a,
            amount_in,
            min_profit: 0,
            fee_lo_ppm: 1_000,
            fee_hi_ppm: 1_000,
            protocol_share_bps: 2_000,
            tick_spacing: SPACING,
        }
    }

    fn run(
        lo: &mut Shard,
        hi: &mut Shard,
        p: RebalanceParams,
    ) -> Result<RebalanceOutcome, AmmError> {
        let (mut lp, mut hp) = (lo.pool, hi.pool);
        // A only ever gets bought on `lo` and sold on `hi`.
        let mut lr = refs(&lp, &mut lo.arrays, false);
        let mut hr = refs(&hp, &mut hi.arrays, true);
        let r = rebalance(&mut lp, &mut lr, &mut hp, &mut hr, p);
        drop((lr, hr));
        if r.is_ok() {
            lo.pool = lp;
            hi.pool = hp;
        }
        r
    }

    /// Same result as two plain swaps with the same amounts and limits.
    #[test]
    fn equals_two_plain_swaps_in_both_directions() {
        for input_is_a in [false, true] {
            let (mut lo, mut hi) = (Shard::at_tick(-300, 1 << 50), Shard::at_tick(300, 1 << 50));
            let (mut lo2, mut hi2) = (Shard::at_tick(-300, 1 << 50), Shard::at_tick(300, 1 << 50));
            let amount = 1u64 << 40;
            let out = run(&mut lo, &mut hi, params(input_is_a, amount)).unwrap();
            assert!(out.profit > 0);
            assert_eq!(out.profit, out.second.amount_out - amount);
            assert!(
                lo.pool.sqrt_price < hi.pool.sqrt_price,
                "prices must not cross"
            );
            assert!(
                lo.pool.sqrt_price > lo2.pool.sqrt_price
                    && hi.pool.sqrt_price < hi2.pool.sqrt_price
            );

            let plain = |s: &mut Shard, a_to_b: bool, amount_in: u64, limit: u128, fee: u32| {
                let mut r = refs(&s.pool, &mut s.arrays, a_to_b);
                swap(
                    &mut s.pool,
                    &mut r,
                    SwapParams {
                        a_to_b,
                        amount_in,
                        sqrt_price_limit: limit,
                        fee_rate_ppm: fee,
                        protocol_share_bps: 2_000,
                        tick_spacing: SPACING,
                    },
                )
                .unwrap()
            };
            let (hi_limit, lo_limit) = (hi2.pool.sqrt_price, lo2.pool.sqrt_price);
            if input_is_a {
                let f = plain(&mut hi2, true, amount, lo_limit, 1_000);
                let lo_after = hi2.pool.sqrt_price;
                let s = plain(&mut lo2, false, f.amount_out, lo_after, 1_000);
                assert_eq!((f, s), (out.first, out.second));
            } else {
                let f = plain(&mut lo2, false, amount, hi_limit, 1_000);
                let lo_after = lo2.pool.sqrt_price;
                let s = plain(&mut hi2, true, f.amount_out, lo_after, 1_000);
                assert_eq!((f, s), (out.first, out.second));
            }
            assert_eq!((lo.pool, hi.pool), (lo2.pool, hi2.pool));
            assert_eq!(lo.arrays, lo2.arrays);
            assert_eq!(hi.arrays, hi2.arrays);
        }
    }

    #[test]
    fn refuses_equal_prices_no_profit_and_overshoot() {
        let (mut a, mut b) = (Shard::at_tick(0, 1 << 50), Shard::at_tick(0, 1 << 50));
        assert_eq!(
            run(&mut a, &mut b, params(false, 1 << 30)),
            Err(AmmError::RebalanceNoPriceGap)
        );
        // Gap smaller than two fees: nothing to gain.
        let (mut lo, mut hi) = (Shard::at_tick(-5, 1 << 50), Shard::at_tick(5, 1 << 50));
        assert_eq!(
            run(&mut lo, &mut hi, params(false, 1 << 30)),
            Err(AmmError::RebalanceUnprofitable)
        );
        // Too large: the first leg would pass the other shard's price.
        let (mut lo, mut hi) = (Shard::at_tick(-300, 1 << 50), Shard::at_tick(300, 1 << 50));
        assert_eq!(
            run(&mut lo, &mut hi, params(false, 1 << 60)),
            Err(AmmError::PriceLimitReached)
        );
        // A profit floor above what is available.
        let p = RebalanceParams {
            min_profit: u64::MAX,
            ..params(false, 1 << 40)
        };
        assert_eq!(
            run(&mut lo, &mut hi, p),
            Err(AmmError::RebalanceUnprofitable)
        );
    }

    /// Random gaps, sizes and fees: whenever it succeeds the prices have
    /// moved toward each other without crossing and the profit is exact.
    #[test]
    fn random_rebalances_converge() {
        let mut x: u64 = 0x5eed_0006_0ba1_a2ce;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut ok = 0;
        for _ in 0..400 {
            let gap = 1 + (next() % 2_000) as i32;
            let base = (next() % 2_000) as i32 - 1_000;
            let l = 1u128 << (40 + next() % 12);
            let (mut lo, mut hi) = (Shard::at_tick(base - gap, l), Shard::at_tick(base + gap, l));
            let (lo0, hi0) = (lo.pool.sqrt_price, hi.pool.sqrt_price);
            let p = RebalanceParams {
                input_is_a: next() % 2 == 0,
                amount_in: 1 + (next() >> (next() % 40 + 4)),
                fee_lo_ppm: (next() % 8_000) as u32,
                fee_hi_ppm: (next() % 8_000) as u32,
                ..params(false, 0)
            };
            if let Ok(o) = run(&mut lo, &mut hi, p) {
                ok += 1;
                assert!(lo.pool.sqrt_price > lo0 && hi.pool.sqrt_price < hi0);
                assert!(lo.pool.sqrt_price <= hi.pool.sqrt_price);
                assert_eq!(o.first.amount_out, o.second.amount_in);
                assert_eq!(o.profit, o.second.amount_out - p.amount_in);
            } else {
                assert_eq!((lo.pool.sqrt_price, hi.pool.sqrt_price), (lo0, hi0));
            }
        }
        assert!(ok > 50, "only {ok} succeeded");
    }
}
