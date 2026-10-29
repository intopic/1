//! Differential tests: the on-chain math must reproduce every value from the
//! independent Python reference (tools/reference/clmm_vectors.py) exactly.

use serde_json::Value;
use sharded_amm::constants::{MAX_TICK, MIN_TICK};
use sharded_amm::errors::AmmError;
use sharded_amm::math::fee_math::{fees_owed, growth_delta, growth_inside};
use sharded_amm::math::sqrt_price_math::{
    amount_a_delta, amount_b_delta, next_sqrt_price_from_a_input, next_sqrt_price_from_b_input,
};
use sharded_amm::math::swap_step::compute_swap_step;
use sharded_amm::math::tick_math::{
    sqrt_price_at_tick, tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64,
};
use sharded_amm::math::wide::U256;

const VECTORS: &str = include_str!("../../../docs/vectors/clmm_vectors.json");

fn doc() -> Value {
    serde_json::from_str(VECTORS).expect("valid vector file")
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str()
        .unwrap_or_else(|| panic!("{k} missing"))
        .to_owned()
}

fn u128_of(v: &Value, k: &str) -> u128 {
    s(v, k).parse().unwrap()
}

fn u64_of(v: &Value, k: &str) -> u64 {
    s(v, k).parse().unwrap()
}

fn u256_of(v: &Value, k: &str) -> U256 {
    U256::from_dec_str(&s(v, k)).unwrap()
}

fn rows<'a>(d: &'a Value, k: &str) -> &'a Vec<Value> {
    let r = d[k].as_array().unwrap();
    assert!(!r.is_empty(), "{k} is empty");
    r
}

#[test]
fn domain_constants() {
    let d = doc();
    assert_eq!(MIN_SQRT_PRICE_X64, u128_of(&d, "min_sqrt_price_x64"));
    assert_eq!(MAX_SQRT_PRICE_X64, u128_of(&d, "max_sqrt_price_x64"));
}

#[test]
fn tick_prices_match_exact_floor() {
    let d = doc();
    for r in rows(&d, "ticks") {
        let t = r["tick"].as_i64().unwrap() as i32;
        let expected = u128_of(r, "sqrt_price");
        assert_eq!(sqrt_price_at_tick(t).unwrap(), expected, "tick {t}");
        assert_eq!(
            tick_at_sqrt_price(expected).unwrap(),
            t,
            "inverse at tick {t}"
        );
    }
}

#[test]
fn amount_deltas_match() {
    let d = doc();
    for r in rows(&d, "amount_deltas") {
        let (s0, s1, l) = (u128_of(r, "s0"), u128_of(r, "s1"), u128_of(r, "liquidity"));
        assert_eq!(
            amount_a_delta(s0, s1, l, false).unwrap(),
            u256_of(r, "a_down"),
            "{r}"
        );
        assert_eq!(
            amount_a_delta(s0, s1, l, true).unwrap(),
            u256_of(r, "a_up"),
            "{r}"
        );
        assert_eq!(
            amount_b_delta(s0, s1, l, false).unwrap(),
            u256_of(r, "b_down"),
            "{r}"
        );
        assert_eq!(
            amount_b_delta(s0, s1, l, true).unwrap(),
            u256_of(r, "b_up"),
            "{r}"
        );
    }
}

#[test]
fn next_prices_match() {
    let d = doc();
    for r in rows(&d, "next_prices") {
        let (p, l, amt) = (
            u128_of(r, "s"),
            u128_of(r, "liquidity"),
            u64_of(r, "amount"),
        );
        assert_eq!(
            next_sqrt_price_from_a_input(p, l, amt).unwrap(),
            u128_of(r, "next_a"),
            "{r}"
        );
        let b = next_sqrt_price_from_b_input(p, l, amt);
        match s(r, "next_b").as_str() {
            "MathOverflow" => assert_eq!(b, Err(AmmError::MathOverflow)),
            v => assert_eq!(b.unwrap(), v.parse::<u128>().unwrap(), "{r}"),
        }
    }
}

#[test]
fn swap_steps_match() {
    let d = doc();
    let (mut ok, mut err) = (0, 0);
    for r in rows(&d, "swap_steps") {
        let got = compute_swap_step(
            u128_of(r, "s"),
            u128_of(r, "target"),
            u128_of(r, "liquidity"),
            u64_of(r, "gross"),
            r["fee_ppm"].as_u64().unwrap() as u32,
        );
        if let Some(e) = r["error"].as_str() {
            let expected = match e {
                "ZeroLiquidityGap" => AmmError::ZeroLiquidityGap,
                "MathOverflow" => AmmError::MathOverflow,
                other => panic!("unknown error {other}"),
            };
            assert_eq!(got, Err(expected), "{r}");
            err += 1;
            continue;
        }
        let step = got.unwrap_or_else(|e| panic!("{e:?} for {r}"));
        assert_eq!(step.sqrt_price_next, u128_of(r, "next"), "{r}");
        assert_eq!(step.input_net, u64_of(r, "input_net"), "{r}");
        assert_eq!(step.fee_nominal, u64_of(r, "fee_nominal"), "{r}");
        assert_eq!(step.rounding_dust, u64_of(r, "rounding_dust"), "{r}");
        assert_eq!(step.output, u64_of(r, "output"), "{r}");
        assert_eq!(step.reached_target, r["reached"].as_bool().unwrap(), "{r}");
        ok += 1;
    }
    assert!(
        ok > 100 && err > 10,
        "vector mix too narrow: {ok} ok, {err} errors"
    );
}

#[test]
fn fee_growth_matches() {
    let d = doc();
    for r in rows(&d, "fee_growth") {
        let delta = growth_delta(u64_of(r, "lp_fee"), u128_of(r, "liquidity")).unwrap();
        assert_eq!(delta, u256_of(r, "growth_delta"), "{r}");
        let owed = fees_owed(u128_of(r, "position_liquidity"), delta, U256::zero());
        match s(r, "owed").as_str() {
            "MathOverflow" => assert_eq!(owed, Err(AmmError::MathOverflow)),
            v => assert_eq!(owed.unwrap(), v.parse::<u64>().unwrap(), "{r}"),
        }
    }
}

/// Audit F-03: the `[u64; 4]` fee fields are U256 limbs, least significant
/// first, and Borsh writes them as 32 little-endian bytes.
#[test]
fn u256_limbs_match_borsh_layout() {
    use anchor_lang::AnchorSerialize;
    let d = doc();
    for r in rows(&d, "u256_limbs") {
        let limbs: [u64; 4] =
            std::array::from_fn(|i| r["limbs"][i].as_str().unwrap().parse().unwrap());
        let v = U256(limbs);
        assert_eq!(v, u256_of(r, "value"), "{r}");
        let mut bytes = Vec::new();
        limbs.serialize(&mut bytes).unwrap();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, s(r, "borsh_hex"), "{r}");
    }
}

/// Audit F-03: Q128 inside growth modulo 2^256 and the owed amount since a
/// checkpoint, on reference random walks that include negative `inside`
/// values and wrapped differences.
#[test]
fn fee_inside_modular_matches() {
    let d = doc();
    let rows = rows(&d, "fee_inside");
    let (mut negative, mut wrapped) = (0, 0);
    for r in rows {
        let i = |k: &str| r[k].as_i64().unwrap() as i32;
        let (global, inside, checkpoint) = (
            u256_of(r, "global"),
            u256_of(r, "inside"),
            u256_of(r, "checkpoint"),
        );
        let got = growth_inside(
            i("tick_lower"),
            i("tick_upper"),
            i("tick_current"),
            global,
            u256_of(r, "outside_lower"),
            u256_of(r, "outside_upper"),
        )
        .unwrap();
        assert_eq!(got, inside, "{r}");
        negative += usize::from(inside > global);
        wrapped += usize::from(checkpoint > inside);
        let owed = fees_owed(u128_of(r, "position_liquidity"), inside, checkpoint);
        match s(r, "owed").as_str() {
            "MathOverflow" => assert_eq!(owed, Err(AmmError::MathOverflow)),
            v => assert_eq!(owed.unwrap(), v.parse::<u64>().unwrap(), "{r}"),
        }
    }
    assert!(negative > 0 && wrapped > 0, "{negative} {wrapped}");
}

/// Every tick of the domain against the exact table written by
/// `tools/reference/all_tick_prices.py` (run by the stress workflow).
#[test]
fn full_tick_table() {
    let Ok(path) = std::env::var("AMM_FULL_TICK_TABLE") else {
        println!("AMM_FULL_TICK_TABLE not set; skipped");
        return;
    };
    let bytes = std::fs::read(path).expect("read tick table");
    let n = (2 * MAX_TICK + 1) as usize;
    assert_eq!(bytes.len(), 16 * n);
    for (i, chunk) in bytes.chunks_exact(16).enumerate() {
        let t = MIN_TICK + i as i32;
        let want = u128::from_le_bytes(chunk.try_into().unwrap());
        assert_eq!(sqrt_price_at_tick(t).unwrap(), want, "tick {t}");
        assert_eq!(tick_at_sqrt_price(want).unwrap(), t, "inverse {t}");
        if t > MIN_TICK {
            assert_eq!(tick_at_sqrt_price(want - 1).unwrap(), t - 1, "below {t}");
        }
    }
    println!("all {n} ticks exact");
}
