//! Exact-in swap across initialized ticks of one shard (numeric-spec section 5).
//!
//! The whole input is consumed or the swap fails: there are no partial fills.
//! `arrays[0]` must contain the current tick and each following array must be
//! the adjacent one in the swap direction. A step that needs a price beyond
//! the last array fails with `TickArraysExhausted`.

use crate::constants::{MAX_SWAP_STEPS, MAX_TICK, MIN_TICK};
use crate::errors::AmmError;
use crate::math::fee_math::{add_growth, cross_outside, growth_delta, split_fee};
use crate::math::liquidity_math::add_delta;
use crate::math::swap_step::compute_swap_step;
use crate::math::tick_math::{
    sqrt_price_at_tick, tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64,
};
use crate::math::wide::U256;
use crate::state::{array_span, array_start_for, TickArray};

use super::Pool;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapOutcome {
    pub amount_in: u64,
    pub amount_out: u64,
    /// Nominal fee plus rounding dust, split into protocol and LP parts.
    pub total_fee: u64,
    pub protocol_fee: u64,
    pub rounding_dust: u64,
    pub steps: u32,
    pub ticks_crossed: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct SwapParams {
    pub a_to_b: bool,
    pub amount_in: u64,
    /// 0 means the domain bound in the swap direction.
    pub sqrt_price_limit: u128,
    pub fee_rate_ppm: u32,
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
}

fn check_array_sequence(
    arrays: &[&mut TickArray],
    pool: &Pool,
    p: &SwapParams,
) -> Result<(), AmmError> {
    let first = arrays.first().ok_or(AmmError::TickArrayMismatch)?;
    if first.start_tick_index != array_start_for(pool.tick, p.tick_spacing) {
        return Err(AmmError::TickArrayMismatch);
    }
    let span = array_span(p.tick_spacing);
    for w in arrays.windows(2) {
        let expected = if p.a_to_b {
            w[0].start_tick_index - span
        } else {
            w[0].start_tick_index + span
        };
        if w[1].start_tick_index != expected || w[1].shard != w[0].shard {
            return Err(AmmError::TickArrayMismatch);
        }
    }
    Ok(())
}

pub fn swap(
    pool: &mut Pool,
    arrays: &mut [&mut TickArray],
    p: SwapParams,
) -> Result<SwapOutcome, AmmError> {
    if p.amount_in == 0 {
        return Err(AmmError::ZeroAmount);
    }
    // Limits are strictly inside the domain, so the price never reaches
    // MIN/MAX and the current tick stays within [MIN_TICK, MAX_TICK).
    let limit = match (p.sqrt_price_limit, p.a_to_b) {
        (0, true) => MIN_SQRT_PRICE_X64 + 1,
        (0, false) => MAX_SQRT_PRICE_X64 - 1,
        (l, _) => l,
    };
    let limit_ok = if p.a_to_b {
        limit > MIN_SQRT_PRICE_X64 && limit < pool.sqrt_price
    } else {
        limit > pool.sqrt_price && limit < MAX_SQRT_PRICE_X64
    };
    if !limit_ok {
        return Err(AmmError::InvalidPriceLimit);
    }
    check_array_sequence(arrays, pool, &p)?;

    let mut st = *pool;
    let mut out = SwapOutcome {
        amount_in: p.amount_in,
        ..SwapOutcome::default()
    };
    let mut remaining = p.amount_in;
    let mut ai = 0usize;
    let span = array_span(p.tick_spacing);
    // Crossings are applied after the loop so a failed swap leaves the
    // arrays untouched; each stores the global growth at crossing time.
    let mut crossings: Vec<(usize, i32, U256, U256)> = Vec::new();

    while remaining > 0 {
        out.steps += 1;
        if out.steps > MAX_SWAP_STEPS {
            return Err(AmmError::TooManySwapSteps);
        }
        let array = arrays.get(ai).ok_or(AmmError::TickArraysExhausted)?;
        let found = array.next_initialized(st.tick, p.tick_spacing, p.a_to_b);
        let target_tick = match (found, p.a_to_b) {
            (Some(t), _) => t,
            (None, true) => array.start_tick_index.max(MIN_TICK),
            (None, false) => (array.start_tick_index + span).min(MAX_TICK),
        };
        let tick_price = sqrt_price_at_tick(target_tick)?;
        let target = if p.a_to_b {
            tick_price.max(limit)
        } else {
            tick_price.min(limit)
        };

        let step = compute_swap_step(
            st.sqrt_price,
            target,
            st.liquidity,
            remaining,
            p.fee_rate_ppm,
        )?;
        remaining -= step.gross_consumed();
        out.amount_out = out
            .amount_out
            .checked_add(step.output)
            .ok_or(AmmError::MathOverflow)?;
        out.rounding_dust += step.rounding_dust;
        let fee = step.total_fee();
        if fee > 0 {
            let (protocol, lp) = split_fee(fee, p.protocol_share_bps)?;
            out.total_fee += fee;
            out.protocol_fee += protocol;
            if lp > 0 {
                let g = growth_delta(lp, st.liquidity)?;
                if p.a_to_b {
                    st.growth_a = add_growth(st.growth_a, g)?;
                } else {
                    st.growth_b = add_growth(st.growth_b, g)?;
                }
            }
            if p.a_to_b {
                st.protocol_a = st
                    .protocol_a
                    .checked_add(protocol)
                    .ok_or(AmmError::MathOverflow)?;
            } else {
                st.protocol_b = st
                    .protocol_b
                    .checked_add(protocol)
                    .ok_or(AmmError::MathOverflow)?;
            }
        }
        let moved = step.sqrt_price_next != st.sqrt_price;
        st.sqrt_price = step.sqrt_price_next;

        if step.reached_target && target == tick_price {
            if found.is_some() {
                let t = arrays[ai].tick(target_tick, p.tick_spacing)?;
                let net = if p.a_to_b {
                    t.net().checked_neg().ok_or(AmmError::MathOverflow)?
                } else {
                    t.net()
                };
                st.liquidity = add_delta(st.liquidity, net)?;
                crossings.push((ai, target_tick, st.growth_a, st.growth_b));
                out.ticks_crossed += 1;
                st.tick = if p.a_to_b {
                    target_tick - 1
                } else {
                    target_tick
                };
                if !arrays[ai].contains(st.tick, p.tick_spacing) {
                    ai += 1;
                }
            } else {
                // Array boundary. The boundary tick itself belongs to the next
                // array (moving up) or was already searched (moving down), so
                // `boundary - 1` keeps it eligible for the next search.
                st.tick = target_tick - 1;
                ai += 1;
            }
        } else {
            // Stopped inside an interval or at the caller's limit. Recompute
            // the tick only if the price moved: a zero step right after a
            // downward crossing must keep `tick = crossed - 1`.
            if moved {
                st.tick = tick_at_sqrt_price(st.sqrt_price)?;
            }
            if step.reached_target && remaining > 0 {
                return Err(AmmError::PriceLimitReached);
            }
        }
    }
    if out.amount_out == 0 {
        return Err(AmmError::ZeroOutput);
    }
    // Validate every crossing before writing any of them.
    let mut updates = Vec::with_capacity(crossings.len());
    for &(i, tick, ga, gb) in &crossings {
        let t = arrays[i].tick(tick, p.tick_spacing)?;
        updates.push((
            i,
            tick,
            cross_outside(ga, t.outside_a())?,
            cross_outside(gb, t.outside_b())?,
        ));
    }
    for (i, tick, oa, ob) in updates {
        arrays[i]
            .tick_mut(tick, p.tick_spacing)?
            .set_outside(oa, ob);
    }
    *pool = st;
    Ok(out)
}
