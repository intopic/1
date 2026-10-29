//! Per-shard dynamic fee (ADR-0005).
//!
//! The fee of a swap is fixed when it starts: `begin` rolls the episode
//! forward to the current slot and returns the fee; the swap runs at that
//! fee; `finish` records how far the price moved. Every value is an integer
//! and every intermediate fits `u64`, so the host and the program agree bit
//! for bit and a router can quote with `quote_fee`.
//!
//! The state lives in `ShardState`; the parameters in `MarketConfig` are only
//! read. No account another shard uses is written.

use crate::constants::VOL_UNIT;
use crate::state::{DynamicFeeParams, DynamicFeeState};

/// Volatility of `tick` relative to the episode reference, clamped.
fn accumulate(st: &DynamicFeeState, p: &DynamicFeeParams, spacing: u16, tick: i32) -> u32 {
    let moved = u64::from(tick.abs_diff(st.tick_reference)) * VOL_UNIT / u64::from(spacing.max(1));
    (u64::from(st.vol_reference) + moved).min(u64::from(p.max_vol_accumulator)) as u32
}

/// Rolls the state to `slot` (new episode, decay, per-slot counter) and
/// returns the fee for a swap starting at `tick`.
pub fn begin(
    st: &mut DynamicFeeState,
    p: &DynamicFeeParams,
    base_ppm: u32,
    spacing: u16,
    tick: i32,
    slot: u64,
) -> u32 {
    let elapsed = slot.saturating_sub(st.last_update_slot);
    if elapsed >= u64::from(p.filter_slots) {
        st.tick_reference = tick;
        st.vol_reference = if elapsed < u64::from(p.decay_slots) {
            (u64::from(st.vol_accumulator) * u64::from(p.reduction_bps) / 10_000) as u32
        } else {
            0
        };
    }
    if slot != st.last_update_slot {
        st.slot_swaps = 0;
    }
    if !p.is_enabled(base_ppm) {
        return base_ppm;
    }
    let v = accumulate(st, p, spacing, tick);
    let fee = u64::from(base_ppm)
        + u64::from(p.vol_coeff_ppm) * u64::from(v) / VOL_UNIT
        + u64::from(p.load_coeff_ppm) * u64::from(st.slot_swaps);
    fee.min(u64::from(p.max_fee_rate_ppm)) as u32
}

/// Records a swap that ended at `tick_after` in `slot`.
pub fn finish(
    st: &mut DynamicFeeState,
    p: &DynamicFeeParams,
    spacing: u16,
    tick_after: i32,
    slot: u64,
) {
    st.vol_accumulator = accumulate(st, p, spacing, tick_after);
    st.last_update_slot = slot;
    st.slot_swaps = st.slot_swaps.saturating_add(1);
}

/// The fee a swap would pay if it ran on this shard in `slot`; does not
/// change the state.
pub fn quote_fee(
    st: &DynamicFeeState,
    p: &DynamicFeeParams,
    base_ppm: u32,
    spacing: u16,
    tick: i32,
    slot: u64,
) -> u32 {
    let mut copy = *st;
    begin(&mut copy, p, base_ppm, spacing, tick, slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{MAX_DYNAMIC_COEFF_PPM, MAX_DYNAMIC_FEE_RATE_PPM, MAX_VOL_ACCUMULATOR};

    const BASE: u32 = 300;

    fn params() -> DynamicFeeParams {
        DynamicFeeParams {
            max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM,
            vol_coeff_ppm: 100,
            load_coeff_ppm: 0,
            max_vol_accumulator: 50_000,
            filter_slots: 10,
            decay_slots: 100,
            reduction_bps: 5_000,
        }
    }

    #[test]
    fn disabled_always_charges_the_base() {
        let p = DynamicFeeParams::disabled(BASE);
        let mut st = DynamicFeeState::default();
        for (i, tick) in [0, 6_000, -6_000, 443_000].into_iter().enumerate() {
            assert_eq!(begin(&mut st, &p, BASE, 60, tick, i as u64), BASE);
            finish(&mut st, &p, 60, tick + 600, i as u64);
        }
    }

    #[test]
    fn fee_rises_with_movement_within_an_episode() {
        let p = params();
        let mut st = DynamicFeeState::default();
        // First swap: new episode at tick 0, no history.
        assert_eq!(begin(&mut st, &p, BASE, 60, 0, 1_000), BASE);
        // It moves the price 10 spacings.
        finish(&mut st, &p, 60, 600, 1_000);
        assert_eq!(st.vol_accumulator, 10_000);
        // Next slot, same episode: 10 spacings * 100 ppm = +1000 ppm.
        assert_eq!(begin(&mut st, &p, BASE, 60, 600, 1_001), BASE + 1_000);
        finish(&mut st, &p, 60, 1_200, 1_001);
        assert_eq!(begin(&mut st, &p, BASE, 60, 1_200, 1_002), BASE + 2_000);
        // Moving back toward the reference lowers it again.
        finish(&mut st, &p, 60, 300, 1_002);
        assert_eq!(begin(&mut st, &p, BASE, 60, 300, 1_003), BASE + 500);
    }

    #[test]
    fn episodes_decay_and_reset() {
        let p = params();
        let mut st = DynamicFeeState::default();
        begin(&mut st, &p, BASE, 60, 0, 1_000);
        finish(&mut st, &p, 60, 1_200, 1_000);
        assert_eq!(st.vol_accumulator, 20_000);
        // After the filter period: new episode, half carried over.
        let mut a = st;
        assert_eq!(begin(&mut a, &p, BASE, 60, 1_200, 1_010), BASE + 1_000);
        assert_eq!((a.tick_reference, a.vol_reference), (1_200, 10_000));
        // After the decay period: nothing carried over.
        let mut b = st;
        assert_eq!(begin(&mut b, &p, BASE, 60, 1_200, 1_100), BASE);
        assert_eq!(b.vol_reference, 0);
    }

    #[test]
    fn load_term_counts_swaps_in_the_same_slot() {
        let p = DynamicFeeParams {
            vol_coeff_ppm: 0,
            load_coeff_ppm: 50,
            ..params()
        };
        let mut st = DynamicFeeState::default();
        for k in 0..5u32 {
            assert_eq!(begin(&mut st, &p, BASE, 60, 0, 7), BASE + 50 * k);
            finish(&mut st, &p, 60, 0, 7);
        }
        assert_eq!(begin(&mut st, &p, BASE, 60, 0, 8), BASE, "new slot resets");
    }

    #[test]
    fn quote_matches_begin_and_leaves_state() {
        let p = params();
        let mut st = DynamicFeeState::default();
        begin(&mut st, &p, BASE, 10, -50, 3);
        finish(&mut st, &p, 10, 70, 3);
        let before = st;
        let q = quote_fee(&st, &p, BASE, 10, 70, 5);
        assert_eq!(st, before);
        assert_eq!(q, begin(&mut st, &p, BASE, 10, 70, 5));
    }

    /// Random parameters, ticks and slots: the fee stays in
    /// `[base, max]`, the accumulator under its cap, and nothing overflows.
    #[test]
    fn random_walks_stay_within_bounds() {
        let mut x: u64 = 0x5eed_0005_dfee_0001;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..2_000 {
            let base = (next() % 8_000) as u32;
            let filter = 1 + (next() % 50) as u16;
            let p = DynamicFeeParams {
                max_fee_rate_ppm: base
                    + (next() % u64::from(MAX_DYNAMIC_FEE_RATE_PPM - base + 1)) as u32,
                vol_coeff_ppm: (next() % u64::from(MAX_DYNAMIC_COEFF_PPM + 1)) as u32,
                load_coeff_ppm: (next() % u64::from(MAX_DYNAMIC_COEFF_PPM + 1)) as u32,
                max_vol_accumulator: (next() % u64::from(MAX_VOL_ACCUMULATOR + 1)) as u32,
                filter_slots: filter,
                decay_slots: filter + (next() % 500) as u16,
                reduction_bps: (next() % 10_001) as u16,
            };
            assert_eq!(p.validate(base), Ok(()));
            let spacing = 1 + (next() % 1_000) as u16;
            let mut st = DynamicFeeState::default();
            let (mut tick, mut slot) = (0i32, next() % 1_000_000);
            for _ in 0..50 {
                slot += next() % 30;
                let fee = begin(&mut st, &p, base, spacing, tick, slot);
                assert!(fee >= base && fee <= p.max_fee_rate_ppm.max(base));
                tick = (next() % 887_272) as i32 - 443_636;
                finish(&mut st, &p, spacing, tick, slot);
                assert!(st.vol_accumulator <= p.max_vol_accumulator);
            }
        }
    }
}
