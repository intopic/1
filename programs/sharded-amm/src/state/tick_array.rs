//! `TickArray`: `TICKS_PER_ARRAY` tick slots for one shard, zero-copy.
//!
//! PDA `["ticks", shard, start_tick_index_i32_le]`. An array covers ticks
//! `[start, start + 32 * spacing)`; slot `i` is tick `start + i * spacing`.
//! A tick is initialized when `liquidity_gross > 0`. Arrays are created
//! permissionlessly; swaps must be given every array on their price path.

use anchor_lang::prelude::*;

use crate::constants::{MAX_TICK, MIN_TICK, TICKS_PER_ARRAY};
use crate::errors::AmmError;
use crate::math::wide::U256;

#[zero_copy]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// i128 as little-endian u64 limbs (two's complement).
    pub liquidity_net: [u64; 2],
    /// u128 as little-endian u64 limbs.
    pub liquidity_gross: [u64; 2],
    /// U256 Q128 limbs; checked, `<= fee_growth_global`.
    pub fee_growth_outside_a: [u64; 4],
    pub fee_growth_outside_b: [u64; 4],
}

fn to_limbs(v: u128) -> [u64; 2] {
    [v as u64, (v >> 64) as u64]
}

fn from_limbs(l: [u64; 2]) -> u128 {
    u128::from(l[0]) | (u128::from(l[1]) << 64)
}

impl Tick {
    pub fn net(&self) -> i128 {
        from_limbs(self.liquidity_net) as i128
    }
    pub fn set_net(&mut self, v: i128) {
        self.liquidity_net = to_limbs(v as u128);
    }
    pub fn gross(&self) -> u128 {
        from_limbs(self.liquidity_gross)
    }
    pub fn set_gross(&mut self, v: u128) {
        self.liquidity_gross = to_limbs(v);
    }
    pub fn outside_a(&self) -> U256 {
        U256(self.fee_growth_outside_a)
    }
    pub fn outside_b(&self) -> U256 {
        U256(self.fee_growth_outside_b)
    }
    pub fn set_outside(&mut self, a: U256, b: U256) {
        self.fee_growth_outside_a = a.0;
        self.fee_growth_outside_b = b.0;
    }
    pub fn is_initialized(&self) -> bool {
        self.gross() > 0
    }
}

#[account(zero_copy)]
#[derive(Debug, PartialEq, Eq)]
pub struct TickArray {
    pub shard: Pubkey,
    pub start_tick_index: i32,
    pub initialized_count: u32,
    pub ticks: [Tick; TICKS_PER_ARRAY],
    pub bump: u8,
    pub padding: [u8; 7],
}

/// Ticks covered by one array.
pub fn array_span(tick_spacing: u16) -> i32 {
    TICKS_PER_ARRAY as i32 * i32::from(tick_spacing)
}

/// Start index of the array containing `tick` (floor toward -infinity).
pub fn array_start_for(tick: i32, tick_spacing: u16) -> i32 {
    let span = array_span(tick_spacing);
    tick.div_euclid(span) * span
}

pub fn check_array_start(start: i32, tick_spacing: u16) -> std::result::Result<(), AmmError> {
    let span = array_span(tick_spacing);
    if tick_spacing == 0
        || start.rem_euclid(span) != 0
        || start > MAX_TICK
        || start + span <= MIN_TICK
    {
        return Err(AmmError::InvalidTickArrayStart);
    }
    Ok(())
}

impl TickArray {
    pub fn new(shard: Pubkey, start_tick_index: i32, bump: u8) -> Self {
        Self {
            shard,
            start_tick_index,
            initialized_count: 0,
            ticks: [Tick::default(); TICKS_PER_ARRAY],
            bump,
            padding: [0; 7],
        }
    }

    pub fn contains(&self, tick: i32, tick_spacing: u16) -> bool {
        tick >= self.start_tick_index && tick < self.start_tick_index + array_span(tick_spacing)
    }

    fn slot(&self, tick: i32, tick_spacing: u16) -> std::result::Result<usize, AmmError> {
        let spacing = i32::from(tick_spacing);
        if !self.contains(tick, tick_spacing) || tick.rem_euclid(spacing) != 0 {
            return Err(AmmError::TickArrayMismatch);
        }
        Ok(((tick - self.start_tick_index) / spacing) as usize)
    }

    pub fn tick(&self, tick: i32, tick_spacing: u16) -> std::result::Result<&Tick, AmmError> {
        Ok(&self.ticks[self.slot(tick, tick_spacing)?])
    }

    pub fn tick_mut(
        &mut self,
        tick: i32,
        tick_spacing: u16,
    ) -> std::result::Result<&mut Tick, AmmError> {
        let i = self.slot(tick, tick_spacing)?;
        Ok(&mut self.ticks[i])
    }

    /// Price moving down: the largest initialized tick `<= tick_current`.
    /// Price moving up: the smallest initialized tick `> tick_current`.
    pub fn next_initialized(
        &self,
        tick_current: i32,
        tick_spacing: u16,
        a_to_b: bool,
    ) -> Option<i32> {
        let spacing = i32::from(tick_spacing);
        let start = self.start_tick_index;
        // Slot holding the largest aligned tick <= tick_current, may be outside.
        let pos = (tick_current - start).div_euclid(spacing);
        let n = TICKS_PER_ARRAY as i32;
        if a_to_b {
            let from = pos.min(n - 1);
            (0..=from)
                .rev()
                .find(|&i| self.ticks[i as usize].is_initialized())
        } else {
            let from = (pos + 1).max(0);
            (from..n).find(|&i| self.ticks[i as usize].is_initialized())
        }
        .map(|i| start + i * spacing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_pod_and_sized() {
        assert_eq!(std::mem::size_of::<Tick>(), 96);
        assert_eq!(
            std::mem::size_of::<TickArray>(),
            32 + 4 + 4 + 96 * TICKS_PER_ARRAY + 8
        );
    }

    #[test]
    fn signed_limbs_round_trip() {
        let mut t = Tick::default();
        for v in [0i128, 1, -1, i128::MAX, i128::MIN, -12345678901234567890] {
            t.set_net(v);
            assert_eq!(t.net(), v);
        }
        t.set_gross(u128::MAX);
        assert_eq!(t.gross(), u128::MAX);
    }

    #[test]
    fn starts_floor_toward_negative_infinity() {
        assert_eq!(array_start_for(0, 60), 0);
        assert_eq!(array_start_for(1919, 60), 0);
        assert_eq!(array_start_for(1920, 60), 1920);
        assert_eq!(array_start_for(-1, 60), -1920);
        assert_eq!(array_start_for(-1920, 60), -1920);
        assert_eq!(array_start_for(-1921, 60), -3840);
        assert!(check_array_start(-1920, 60).is_ok());
        assert_eq!(
            check_array_start(60, 60),
            Err(AmmError::InvalidTickArrayStart)
        );
        // Arrays at the edges of the domain still cover valid ticks.
        let lowest = array_start_for(MIN_TICK, 60);
        let highest = array_start_for(MAX_TICK, 60);
        assert!(check_array_start(lowest, 60).is_ok() && check_array_start(highest, 60).is_ok());
        assert!(check_array_start(highest + 1920, 60).is_err());
        assert!(check_array_start(lowest - 1920, 60).is_err());
    }

    #[test]
    fn search_respects_direction_and_bounds() {
        let mut a = TickArray::new(Pubkey::default(), -320, 0);
        for t in [-320, -100, 0, 300] {
            a.tick_mut(t, 20).unwrap().set_gross(1);
        }
        assert_eq!(a.tick_mut(5, 20), Err(AmmError::TickArrayMismatch));
        assert_eq!(a.tick_mut(320, 20), Err(AmmError::TickArrayMismatch));
        // Down: <= current.
        assert_eq!(a.next_initialized(0, 20, true), Some(0));
        assert_eq!(a.next_initialized(-1, 20, true), Some(-100));
        assert_eq!(a.next_initialized(-321, 20, true), None);
        assert_eq!(a.next_initialized(10_000, 20, true), Some(300));
        // Up: > current.
        assert_eq!(a.next_initialized(0, 20, false), Some(300));
        assert_eq!(a.next_initialized(-101, 20, false), Some(-100));
        assert_eq!(a.next_initialized(-10_000, 20, false), Some(-320));
        assert_eq!(a.next_initialized(300, 20, false), None);
        // Unaligned current tick between slots.
        assert_eq!(a.next_initialized(-95, 20, true), Some(-100));
        assert_eq!(a.next_initialized(-105, 20, false), Some(-100));
    }
}
