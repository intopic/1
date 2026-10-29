//! R-01 (mainnet analysis): every swap of a random workload on the real SBF
//! program is compared bit for bit with an independent Python reference of
//! the whole swap (`tools/reference/swap_reference.py`, written from
//! docs/numeric-spec.md): amounts, fees, protocol fees, final price, tick,
//! liquidity, fee growth and every crossed tick, or the same error.
//!
//! Markets cover tick spacings 1, 10, 60 and 200, fees 0.01% to 1% and
//! protocol shares 0 to 50%. `AMM_SVM_CASES` (swaps per market) and
//! `AMM_SVM_SEED` scale and seed it; `python3` must be on the path.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::*;
use serde_json::{json, Value};
use sharded_amm::math::tick_math::sqrt_price_at_tick;
use sharded_amm::math::wide::U256;
use sharded_amm::state::{array_span, MarketParams, TickArray};
use solana_transaction_error::TransactionError;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn log_amount(&mut self, bits: u32) -> u64 {
        let b = 1 + self.below(u64::from(bits)) as u32;
        (self.next() >> (64 - b)).max(1)
    }
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn u256(limbs: [u64; 4]) -> String {
    U256(limbs).to_string()
}

fn compute_budget(units: u32) -> solana_instruction::Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    solana_instruction::Instruction {
        program_id: anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data,
    }
}

/// The shard and swap as the reference reads them.
fn snapshot(
    env: &Env,
    m: &Market,
    id: u8,
    args: &sharded_amm::instructions::SwapArgs,
    arrays: &[(Pubkey, TickArray)],
) -> Value {
    let cfg = env.market_state(m);
    let st = env.shard_state(m, id);
    let spacing = i32::from(cfg.params.tick_spacing);
    let mut ticks = Vec::new();
    for (_, a) in arrays {
        for (i, t) in a.ticks.iter().enumerate() {
            if t.gross() > 0 {
                ticks.push(json!({
                    "index": a.start_tick_index + i as i32 * spacing,
                    "net": t.net().to_string(),
                    "out_a": t.outside_a().to_string(),
                    "out_b": t.outside_b().to_string(),
                }));
            }
        }
    }
    json!({
        "a_to_b": args.a_to_b,
        "amount_in": args.amount_in.to_string(),
        "limit": args.sqrt_price_limit_x64.to_string(),
        "min_amount_out": args.min_amount_out.to_string(),
        "max_rounding_dust": args.max_rounding_dust.to_string(),
        "fee_rate_ppm": cfg.params.fee_rate_ppm,
        "protocol_share_bps": cfg.params.protocol_share_bps,
        "tick_spacing": cfg.params.tick_spacing,
        "sqrt_price": st.sqrt_price_x64.to_string(),
        "tick": st.tick_current,
        "liquidity": st.liquidity.to_string(),
        "growth_a": u256(st.fee_growth_global_a_x128),
        "growth_b": u256(st.fee_growth_global_b_x128),
        "protocol_a": st.protocol_fees_a.to_string(),
        "protocol_b": st.protocol_fees_b.to_string(),
        "array_starts": arrays.iter().map(|(_, a)| a.start_tick_index).collect::<Vec<_>>(),
        "ticks": ticks,
    })
}

/// What the program did, in the reference's terms.
fn observed(
    env: &Env,
    m: &Market,
    id: u8,
    before: &[(Pubkey, TickArray)],
    debit: u64,
    credit: u64,
    r: &Result<u64, TransactionError>,
) -> Value {
    if let Err(e) = r {
        let code = match e {
            TransactionError::InstructionError(_, InstructionError::Custom(c)) => *c,
            other => panic!("non-program failure {other:?}"),
        };
        let name = amm_router::recovery::decode(code)
            .map(|e| format!("{e:?}"))
            .unwrap_or_else(|| format!("code {code}"));
        return json!({ "err": name });
    }
    let cfg = env.market_state(m);
    let spacing = i32::from(cfg.params.tick_spacing);
    let st = env.shard_state(m, id);
    let mut crossed = Vec::new();
    for (k, a0) in before {
        let a1 = env.tick_array(k);
        for (i, (t0, t1)) in a0.ticks.iter().zip(a1.ticks.iter()).enumerate() {
            if t0 != t1 {
                crossed.push(json!({
                    "index": a0.start_tick_index + i as i32 * spacing,
                    "out_a": t1.outside_a().to_string(),
                    "out_b": t1.outside_b().to_string(),
                }));
            }
        }
    }
    crossed.sort_by_key(|v| v["index"].as_i64());
    let pa = st.protocol_fees_a;
    let pb = st.protocol_fees_b;
    json!({ "ok": {
        "amount_in": debit.to_string(),
        "amount_out": credit.to_string(),
        "sqrt_price": st.sqrt_price_x64.to_string(),
        "tick": st.tick_current,
        "liquidity": st.liquidity.to_string(),
        "growth_a": u256(st.fee_growth_global_a_x128),
        "growth_b": u256(st.fee_growth_global_b_x128),
        "protocol_a": pa.to_string(),
        "protocol_b": pb.to_string(),
        "crossed": crossed,
    }})
}

/// Runs the reference over all cases in one process.
fn reference(cases: &[Value]) -> Vec<Value> {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/reference/swap_reference.py"
    );
    let mut child = Command::new("python3")
        .args(["-I", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3 on the path");
    // Write from another thread: the reference answers line by line, so a
    // large batch would fill both pipes and block if written first.
    let mut stdin = child.stdin.take().unwrap();
    let input: Vec<String> = cases.iter().map(|c| c.to_string()).collect();
    let writer = std::thread::spawn(move || {
        for c in input {
            writeln!(stdin, "{c}").unwrap();
        }
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(out.status.success(), "reference failed");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The fields both sides report. Fee totals are not observable on chain
/// beyond growth and protocol counters, so they are compared through those.
fn comparable(v: &Value) -> Value {
    match v.get("ok") {
        Some(ok) => {
            let mut o = ok.clone();
            let m = o.as_object_mut().unwrap();
            for k in ["total_fee", "protocol_fee", "rounding_dust"] {
                m.remove(k);
            }
            json!({ "ok": o })
        }
        None => v.clone(),
    }
}

fn run_market(params: MarketParams, rng: &mut Rng, swaps: u64) -> (u64, u64, u64) {
    let mut env = Env::new();
    let (m, _) = env.open_seeded_market_with(params);
    let spacing = i32::from(params.tick_spacing);
    let span = array_span(params.tick_spacing);
    let id = 0u8;
    env.ensure_arrays(&m, id, -4 * span, 4 * span - 1);
    let lp = env.user(&m, u64::MAX / 4);
    let trader = env.user(&m, u64::MAX / 4);
    // Many narrow positions so swaps cross ticks; one wide one so most
    // swaps do not stop at a liquidity gap.
    env.add_liquidity(&m, &lp, id, -4 * span, 4 * span - spacing, 1 << 34)
        .unwrap();
    for _ in 0..24 {
        let c = rng.below(160) as i32 - 80;
        let w = 1 + rng.below(16) as i32;
        let (lower, upper) = ((c - w) * spacing, (c + w) * spacing);
        if lower < -4 * span || upper > 4 * span - spacing {
            continue;
        }
        let _ = env.add_liquidity(&m, &lp, id, lower, upper, 1u128 << (26 + rng.below(16)));
    }
    let (mut cases, mut seen) = (Vec::new(), Vec::new());
    for _ in 0..swaps {
        let a_to_b = rng.below(2) == 0;
        let l = env.shard_state(&m, id).liquidity;
        let amount = if l == 0 || rng.below(6) == 0 {
            rng.log_amount(48)
        } else {
            ((l >> (2 + rng.below(23))) as u64)
                .saturating_add(rng.below(1000))
                .max(1)
        };
        let mut args = env.swap_args(a_to_b, amount);
        if rng.below(5) == 0 {
            let t = env.shard_state(&m, id).tick_current;
            let d = (rng.below(3000) as i32 - 600) * if a_to_b { -1 } else { 1 };
            args.sqrt_price_limit_x64 =
                sqrt_price_at_tick((t + d).clamp(-443_000, 443_000)).unwrap();
        }
        if rng.below(10) == 0 {
            args.min_amount_out = rng.log_amount(40);
        }
        if rng.below(10) == 0 {
            args.max_rounding_dust = rng.below(2);
        }
        let keys = env.swap_arrays(&m, id, a_to_b);
        let arrays: Vec<(Pubkey, TickArray)> =
            keys.iter().map(|k| (*k, env.tick_array(k))).collect();
        cases.push(snapshot(&env, &m, id, &args, &arrays));
        let (tin, tout) = if a_to_b {
            (trader.token_a, trader.token_b)
        } else {
            (trader.token_b, trader.token_a)
        };
        let before = (env.balance(&tin), env.balance(&tout));
        let i = env.ix_swap(&m, &trader, id, args, &keys);
        let kp = trader.kp.insecure_clone();
        let r = env.send(&[compute_budget(1_400_000), i], &[&kp]);
        let after = (env.balance(&tin), env.balance(&tout));
        seen.push(observed(
            &env,
            &m,
            id,
            &arrays,
            before.0 - after.0,
            after.1 - before.1,
            &r,
        ));
    }
    let want = reference(&cases);
    assert_eq!(want.len(), seen.len());
    let (mut ok, mut err, mut crossed) = (0, 0, 0);
    for (i, (w, s)) in want.iter().zip(&seen).enumerate() {
        assert_eq!(
            comparable(w),
            comparable(s),
            "{params:?} swap {i}\ncase: {}",
            cases[i]
        );
        match w.get("ok") {
            Some(o) => {
                ok += 1;
                crossed += o["crossed"].as_array().unwrap().len() as u64;
            }
            None => err += 1,
        }
    }
    (ok, err, crossed)
}

#[test]
fn every_swap_matches_the_independent_reference() {
    let swaps = env_u64("AMM_SVM_CASES", 400) / 2;
    let mut rng = Rng(env_u64("AMM_SVM_SEED", 0x5eed_0101) | 1);
    let mut totals = (0, 0, 0);
    for (spacing, fee, share) in [
        (60u16, 3_000u32, 2_000u16),
        (1, 100, 1_600),
        (10, 500, 1_600),
        (200, 10_000, 5_000),
        (10, 1_000, 0),
    ] {
        let params = MarketParams {
            fee_rate_ppm: fee,
            protocol_share_bps: share,
            tick_spacing: spacing,
        };
        let (ok, err, crossed) = run_market(params, &mut rng, swaps.max(20));
        println!("spacing {spacing} fee {fee} share {share}: {ok} ok, {err} errors, {crossed} ticks crossed");
        totals = (totals.0 + ok, totals.1 + err, totals.2 + crossed);
    }
    println!(
        "STATS r01 ok={} err={} crossed={}",
        totals.0, totals.1, totals.2
    );
    assert!(
        totals.0 > totals.1 && totals.2 > 0,
        "workload too easy: {totals:?}"
    );
}
