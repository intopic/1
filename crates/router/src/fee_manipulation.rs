//! Can a trader game the dynamic fee (ADR-0005)? Host tests on the
//! program's own engine, through the router's books, so every swap pays the
//! fee the program would charge.
//!
//! - A round trip never pays, with or without the dynamic part.
//! - A sandwich never earns more with the dynamic fee than with the fixed
//!   fee at the same base: the victim and the back-run pay the raised fee.
//! - Moving the price and bringing it back in one episode does not leave the
//!   fee raised for the next trader: the accumulator measures the distance
//!   from the episode's reference, not the path.

use crate::tests::{books, cfg};
use crate::{next, Book, MarketCfg};
use sharded_amm::constants::MAX_DYNAMIC_FEE_RATE_PPM;
use sharded_amm::state::DynamicFeeParams;

const SLOT: u64 = 1_000;

fn dynamic() -> MarketCfg {
    MarketCfg {
        dynamic: DynamicFeeParams {
            max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM,
            // +0.06% per tick spacing moved: steeper than the simulated
            // settings, so the effects are large.
            vol_coeff_ppm: 600,
            load_coeff_ppm: 0,
            max_vol_accumulator: 10_000_000,
            filter_slots: 10,
            decay_slots: 100,
            reduction_bps: 5_000,
        },
        ..cfg()
    }
}

fn book() -> Book {
    books(1, 1 << 40).remove(0)
}

/// Attacker sells `x` of A, the victim sells `v` of A, the attacker sells
/// back everything it got; returns the attacker's A after minus before.
fn sandwich(m: &MarketCfg, x: u64, v: u64) -> i128 {
    let mut b = book();
    let got = b.apply(m, SLOT, true, x, 0).unwrap().amount_out;
    if v > 0 {
        b.apply(m, SLOT, true, v, 0).unwrap();
    }
    let back = b.apply(m, SLOT, false, got, 0).unwrap().amount_out;
    i128::from(back) - i128::from(x)
}

fn amounts(seed: u64, n: usize) -> Vec<(u64, u64)> {
    let mut r = seed;
    (0..n)
        .map(|_| {
            let x = 1 + next(&mut r) % (1 << (20 + next(&mut r) % 16));
            let v = 1 + next(&mut r) % (1 << (20 + next(&mut r) % 16));
            (x, v)
        })
        .collect()
}

#[test]
fn round_trips_never_pay() {
    for m in [cfg(), dynamic()] {
        for (x, _) in amounts(1, 400) {
            assert!(sandwich(&m, x, 0) < 0, "round trip of {x} paid");
        }
    }
}

#[test]
fn the_dynamic_fee_never_makes_a_sandwich_better() {
    let mut worse = 0;
    for (x, v) in amounts(2, 400) {
        let fixed = sandwich(&cfg(), x, v);
        let dynamic = sandwich(&dynamic(), x, v);
        assert!(
            dynamic <= fixed,
            "x={x} v={v}: dynamic {dynamic} > fixed {fixed}"
        );
        worse += usize::from(dynamic < fixed);
    }
    assert!(
        worse > 300,
        "the raised fee bites in most cases: {worse}/400"
    );
}

#[test]
fn moving_and_returning_does_not_leave_the_fee_raised() {
    let m = dynamic();
    let base = m.base_fee_ppm;
    for (x, _) in amounts(3, 200) {
        let mut b = book();
        let got = b.apply(&m, SLOT, true, x, 0).unwrap().amount_out;
        let raised = b.fee_ppm(&m, SLOT);
        b.apply(&m, SLOT, false, got, 0).unwrap();
        let after = b.fee_ppm(&m, SLOT);
        // Back within one tick spacing of the reference.
        assert!(
            after <= base + m.dynamic.vol_coeff_ppm,
            "x={x}: {after} after a round trip"
        );
        assert!(after <= raised);
    }
    // Holding the price away does raise it, up to the cap.
    let mut b = book();
    b.apply(&m, SLOT, true, 1 << 36, 0).unwrap();
    assert!(b.fee_ppm(&m, SLOT) > base + 10 * m.dynamic.vol_coeff_ppm);
    assert!(b.fee_ppm(&m, SLOT) <= MAX_DYNAMIC_FEE_RATE_PPM);
}
