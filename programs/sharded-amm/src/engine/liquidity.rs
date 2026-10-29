//! Adding and removing a position's liquidity (docs/numeric-spec.md section 4).

use crate::constants::{MAX_TICK, MIN_TICK};
use crate::errors::AmmError;
use crate::math::fee_math::{fees_since, growth_inside, initial_outside};
use crate::math::liquidity_math::{add_delta, amounts_for_liquidity, max_liquidity_per_tick};
use crate::math::tick_math::sqrt_price_at_tick;
use crate::math::wide::U256;
use crate::state::{Position, Tick, TickArray};

use super::Pool;

/// The fields of a `Position` that liquidity changes touch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionState {
    pub lower: i32,
    pub upper: i32,
    pub liquidity: u128,
    pub inside_last_a: U256,
    pub inside_last_b: U256,
    pub owed_a: u64,
    pub owed_b: u64,
    /// Fees already moved to `owed` since `inside_last_*` (`fee_math::fees_since`).
    pub credited_a: u64,
    pub credited_b: u64,
}

impl PositionState {
    pub fn load(p: &Position) -> Self {
        Self {
            lower: p.tick_lower,
            upper: p.tick_upper,
            liquidity: p.liquidity,
            inside_last_a: U256(p.fee_growth_inside_last_a),
            inside_last_b: U256(p.fee_growth_inside_last_b),
            owed_a: p.tokens_owed_a,
            owed_b: p.tokens_owed_b,
            credited_a: p.fees_credited_a,
            credited_b: p.fees_credited_b,
        }
    }

    pub fn store(&self, p: &mut Position) {
        p.liquidity = self.liquidity;
        p.fee_growth_inside_last_a = self.inside_last_a.0;
        p.fee_growth_inside_last_b = self.inside_last_b.0;
        p.tokens_owed_a = self.owed_a;
        p.tokens_owed_b = self.owed_b;
        p.fees_credited_a = self.credited_a;
        p.fees_credited_b = self.credited_b;
    }
}

pub fn check_tick_range(lower: i32, upper: i32, tick_spacing: u16) -> Result<(), AmmError> {
    let spacing = i32::from(tick_spacing);
    if spacing == 0
        || lower >= upper
        || lower < MIN_TICK
        || upper > MAX_TICK
        || lower.rem_euclid(spacing) != 0
        || upper.rem_euclid(spacing) != 0
    {
        return Err(AmmError::InvalidTickRange);
    }
    Ok(())
}

fn find(arrays: &[&mut TickArray], tick: i32, spacing: u16) -> Result<usize, AmmError> {
    arrays
        .iter()
        .position(|a| a.contains(tick, spacing))
        .ok_or(AmmError::TickArrayMismatch)
}

/// New state of one boundary tick after `delta`, and whether it flipped
/// between initialized and empty.
fn updated_tick(
    old: Tick,
    tick: i32,
    pool: &Pool,
    delta: i128,
    is_upper: bool,
    cap: u128,
) -> Result<(Tick, bool), AmmError> {
    let mut t = old;
    let gross_before = old.gross();
    let gross_after = add_delta(gross_before, delta)?;
    if gross_after > cap {
        return Err(AmmError::LiquidityPerTickExceeded);
    }
    let net_delta = if is_upper {
        delta.checked_neg().ok_or(AmmError::MathOverflow)?
    } else {
        delta
    };
    t.set_net(
        old.net()
            .checked_add(net_delta)
            .ok_or(AmmError::MathOverflow)?,
    );
    t.set_gross(gross_after);
    if gross_before == 0 && gross_after > 0 {
        t.set_outside(
            initial_outside(tick, pool.tick, pool.growth_a),
            initial_outside(tick, pool.tick, pool.growth_b),
        );
    }
    Ok((t, (gross_before == 0) != (gross_after == 0)))
}

/// Applies `delta` to a position: updates both boundary ticks, credits fees
/// earned with the liquidity held before the change, updates the active
/// liquidity when the range contains the current tick, and returns the token
/// amounts to move (owed by the user when adding, rounded up; paid to the user
/// when removing, rounded down). `delta == 0` only refreshes fees.
/// Nothing is written unless every check passes.
pub fn modify_position(
    pool: &mut Pool,
    arrays: &mut [&mut TickArray],
    tick_spacing: u16,
    pos: &mut PositionState,
    delta: i128,
) -> Result<(u64, u64), AmmError> {
    check_tick_range(pos.lower, pos.upper, tick_spacing)?;
    let (li, ui) = (
        find(arrays, pos.lower, tick_spacing)?,
        find(arrays, pos.upper, tick_spacing)?,
    );
    let new_position_l =
        add_delta(pos.liquidity, delta).map_err(|_| AmmError::InsufficientPositionLiquidity)?;
    let cap = max_liquidity_per_tick(tick_spacing)?;

    let old_lower = *arrays[li].tick(pos.lower, tick_spacing)?;
    let old_upper = *arrays[ui].tick(pos.upper, tick_spacing)?;
    let (mut lower, mut upper, mut flipped_lower, mut flipped_upper) =
        (old_lower, old_upper, false, false);
    if delta != 0 {
        (lower, flipped_lower) = updated_tick(old_lower, pos.lower, pool, delta, false, cap)?;
        (upper, flipped_upper) = updated_tick(old_upper, pos.upper, pool, delta, true, cap)?;
    }

    let inside_a = growth_inside(
        pos.lower,
        pos.upper,
        pool.tick,
        pool.growth_a,
        lower.outside_a(),
        upper.outside_a(),
    )?;
    let inside_b = growth_inside(
        pos.lower,
        pos.upper,
        pool.tick,
        pool.growth_b,
        lower.outside_b(),
        upper.outside_b(),
    )?;
    let earned_a = fees_since(pos.liquidity, inside_a, pos.inside_last_a, pos.credited_a)?;
    let earned_b = fees_since(pos.liquidity, inside_b, pos.inside_last_b, pos.credited_b)?;
    let owed_a = pos
        .owed_a
        .checked_add(earned_a)
        .ok_or(AmmError::MathOverflow)?;
    let owed_b = pos
        .owed_b
        .checked_add(earned_b)
        .ok_or(AmmError::MathOverflow)?;

    let in_range = pool.tick >= pos.lower && pool.tick < pos.upper;
    let new_pool_l = if in_range {
        add_delta(pool.liquidity, delta)?
    } else {
        pool.liquidity
    };

    let (sa, sb) = (
        sqrt_price_at_tick(pos.lower)?,
        sqrt_price_at_tick(pos.upper)?,
    );
    let amounts = amounts_for_liquidity(pool.sqrt_price, sa, sb, delta.unsigned_abs(), delta > 0)?;

    // Commit. A tick that became empty is cleared so a later re-init starts
    // from the current global growth.
    let mut counts = [0i64; 2];
    for (slot, t, flipped, idx) in [(0, lower, flipped_lower, li), (1, upper, flipped_upper, ui)] {
        let tick = if slot == 0 { pos.lower } else { pos.upper };
        let stored = arrays[idx].tick_mut(tick, tick_spacing)?;
        *stored = if t.gross() == 0 { Tick::default() } else { t };
        if flipped {
            counts[slot] = if t.gross() > 0 { 1 } else { -1 };
        }
    }
    for (slot, idx) in [(0, li), (1, ui)] {
        let c = i64::from(arrays[idx].initialized_count) + counts[slot];
        arrays[idx].initialized_count =
            u32::try_from(c).map_err(|_| AmmError::MathInvariantViolated)?;
    }
    // With unchanged liquidity the checkpoint stays and `credited` grows, so
    // the fee is rounded once over the whole period; a liquidity change
    // starts a new period at `inside`.
    let (last_a, last_b, credited_a, credited_b) = if delta == 0 {
        (
            pos.inside_last_a,
            pos.inside_last_b,
            pos.credited_a
                .checked_add(earned_a)
                .ok_or(AmmError::MathOverflow)?,
            pos.credited_b
                .checked_add(earned_b)
                .ok_or(AmmError::MathOverflow)?,
        )
    } else {
        (inside_a, inside_b, 0, 0)
    };
    pool.liquidity = new_pool_l;
    *pos = PositionState {
        liquidity: new_position_l,
        inside_last_a: last_a,
        inside_last_b: last_b,
        owed_a,
        owed_b,
        credited_a,
        credited_b,
        ..*pos
    };
    Ok(amounts)
}
