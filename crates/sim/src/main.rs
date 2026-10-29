//! ENG-09: compares fee tiers (5, 10, 30 bps), tick spacings (10, 60),
//! range widths and sharding (1 vs 8 shards) against the CPMM baseline with
//! equal LP capital, on the same random price paths and order flow.
//!
//! Model, stated plainly so nobody reads more into the numbers than is there:
//! - The external SOL price follows a driftless log random walk.
//! - Each step, an arbitrageur moves every venue toward the external price
//!   while it is profitable after the fee, and uninformed traders send
//!   random-size orders that an aggregator routes to the best quote.
//! - LP results are measured against holding the initial deposit (HODL);
//!   20% of every fee goes to the protocol, as in ADR-0004.
//! - No gas, priority fees, MEV, JIT liquidity or LP rebalancing.
//!
//! Both venues run the real code: `sharded_amm::engine` for the CLMM and
//! `amm_model::cpmm` for the baseline. Prices are raw units (SOL 9 decimals,
//! USDC 6 decimals). Usage: `eng09 [output.md]`; `SIM_SEEDS`, `SIM_STEPS`
//! scale the run.
//!
//! `eng09 --dynamic [output.md]` compares the fixed fee with the per-shard
//! dynamic fee (ADR-0005), run by the real `engine::dynamic_fee`, at normal
//! and at four times the volatility. A step is one minute = 150 slots; the
//! arbitrageur trades at the start of the step and each uninformed trade
//! lands 3 slots after the previous one.

use amm_model::cpmm;
use anchor_lang::prelude::Pubkey;
use sharded_amm::engine::{dynamic_fee, modify_position, swap, Pool, PositionState, SwapParams};
use sharded_amm::math::liquidity_math::amounts_for_liquidity;
use sharded_amm::math::tick_math::{sqrt_price_at_tick, tick_at_sqrt_price};
use sharded_amm::state::{
    array_span, array_start_for, DynamicFeeParams, DynamicFeeState, TickArray,
};

const PROTOCOL_SHARE_BPS: u16 = 2_000;
/// $150 per SOL in raw units: 150e6 USDC units per 1e9 lamports.
const P0: f64 = 0.15;
const CAPITAL_USD: f64 = 1_000_000.0;
const USDC: f64 = 1e6;
/// One simulation step is one minute.
const SLOTS_PER_STEP: u64 = 150;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn normal(&mut self) -> f64 {
        let (u, v) = (self.unit().max(1e-300), self.unit());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

fn price_of_sqrt(s: u128) -> f64 {
    let x = s as f64 / 18_446_744_073_709_551_616.0;
    x * x
}

trait Venue: Clone {
    /// Marginal price of A in B units, before fees.
    fn price(&self) -> f64;
    fn fee(&self) -> f64;
    /// Executes an exact-in swap; `None` if the venue rejects it.
    fn swap(&mut self, a_to_b: bool, amount: u64) -> Option<u64>;
    /// Moves the venue's clock; only the dynamic fee uses it.
    fn set_slot(&mut self, _slot: u64) {}
    fn quote(&self, a_to_b: bool, amount: u64) -> Option<u64> {
        self.clone().swap(a_to_b, amount)
    }
    /// LP holdings if withdrawn now (principal + LP fees), and fees alone.
    fn lp_exit(&self) -> ((u64, u64), (u64, u64));
}

#[derive(Clone)]
struct Clmm {
    pool: Pool,
    arrays: Vec<TickArray>,
    spacing: u16,
    fee_ppm: u32,
    position: PositionState,
    dynamic: DynamicFeeParams,
    fee_state: DynamicFeeState,
    slot: u64,
}

impl Clmm {
    /// One LP position over `[p0 / (1 + w), p0 * (1 + w)]` worth `capital` USD.
    fn new(fee_ppm: u32, spacing: u16, width: f64, capital: f64) -> (Self, (u64, u64)) {
        let sp = i32::from(spacing);
        let s0 = (P0.sqrt() * 18_446_744_073_709_551_616.0) as u128;
        let t0 = tick_at_sqrt_price(s0).unwrap();
        let dt = ((1.0 + width).ln() / 1.0001f64.ln()) as i32;
        let lower = (t0 - dt).div_euclid(sp) * sp;
        let upper = (t0 + dt).div_euclid(sp) * sp + sp;
        let span = array_span(spacing);
        let first = array_start_for(lower, spacing) - 3 * span;
        let last = array_start_for(upper, spacing) + 3 * span;
        let arrays = (0..=((last - first) / span))
            .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
            .collect();
        let mut v = Self {
            pool: Pool::at_price(s0).unwrap(),
            arrays,
            spacing,
            fee_ppm,
            dynamic: DynamicFeeParams::disabled(fee_ppm),
            fee_state: DynamicFeeState::default(),
            slot: 0,
            position: PositionState {
                lower,
                upper,
                liquidity: 0,
                inside_last_a: Default::default(),
                inside_last_b: Default::default(),
                owed_a: 0,
                owed_b: 0,
                credited_a: 0,
                credited_b: 0,
            },
        };
        let (sa, sb) = (
            sqrt_price_at_tick(lower).unwrap(),
            sqrt_price_at_tick(upper).unwrap(),
        );
        let unit = 1u128 << 40;
        let (a, b) = amounts_for_liquidity(s0, sa, sb, unit, true).unwrap();
        let value = (a as f64 * P0 + b as f64) / USDC;
        let liquidity = (unit as f64 * capital / value) as u128;
        let mut refs: Vec<&mut TickArray> = v.arrays.iter_mut().collect();
        let used = modify_position(
            &mut v.pool,
            &mut refs,
            spacing,
            &mut v.position,
            liquidity as i128,
        )
        .unwrap();
        (v, used)
    }
}

impl Venue for Clmm {
    fn price(&self) -> f64 {
        price_of_sqrt(self.pool.sqrt_price)
    }
    fn fee(&self) -> f64 {
        let rate = dynamic_fee::quote_fee(
            &self.fee_state,
            &self.dynamic,
            self.fee_ppm,
            self.spacing,
            self.pool.tick,
            self.slot,
        );
        f64::from(rate) / 1e6
    }
    fn set_slot(&mut self, slot: u64) {
        self.slot = slot;
    }
    fn swap(&mut self, a_to_b: bool, amount: u64) -> Option<u64> {
        let start = array_start_for(self.pool.tick, self.spacing);
        let idx = self
            .arrays
            .iter()
            .position(|a| a.start_tick_index == start)?;
        let mut refs: Vec<&mut TickArray> = if a_to_b {
            self.arrays[..=idx].iter_mut().rev().take(3).collect()
        } else {
            self.arrays[idx..].iter_mut().take(3).collect()
        };
        let mut fee_state = self.fee_state;
        let fee_rate_ppm = dynamic_fee::begin(
            &mut fee_state,
            &self.dynamic,
            self.fee_ppm,
            self.spacing,
            self.pool.tick,
            self.slot,
        );
        let p = SwapParams {
            a_to_b,
            amount_in: amount,
            sqrt_price_limit: 0,
            fee_rate_ppm,
            protocol_share_bps: PROTOCOL_SHARE_BPS,
            tick_spacing: self.spacing,
        };
        let out = swap(&mut self.pool, &mut refs, p).ok()?;
        dynamic_fee::finish(
            &mut fee_state,
            &self.dynamic,
            self.spacing,
            self.pool.tick,
            self.slot,
        );
        self.fee_state = fee_state;
        Some(out.amount_out)
    }
    fn lp_exit(&self) -> ((u64, u64), (u64, u64)) {
        let mut v = self.clone();
        let mut refs: Vec<&mut TickArray> = v.arrays.iter_mut().collect();
        let l = v.position.liquidity as i128;
        let (a, b) = modify_position(&mut v.pool, &mut refs, v.spacing, &mut v.position, -l)
            .expect("full exit");
        let fees = (v.position.owed_a, v.position.owed_b);
        ((a + fees.0, b + fees.1), fees)
    }
}

#[derive(Clone)]
struct Cpmm {
    pool: cpmm::Pool,
    fee_ppm: u32,
    shares: u64,
    initial: (u64, u64),
}

impl Cpmm {
    fn new(fee_ppm: u32, capital: f64) -> (Self, (u64, u64)) {
        let mut pool = cpmm::Pool::new(fee_ppm, PROTOCOL_SHARE_BPS).unwrap();
        let b = (capital / 2.0 * USDC) as u64;
        let a = (b as f64 / P0) as u64;
        let d = pool.deposit(a, b, 0).unwrap();
        let used = (d.used_x, d.used_y);
        (
            Self {
                pool,
                fee_ppm,
                shares: d.shares,
                initial: used,
            },
            used,
        )
    }
}

impl Venue for Cpmm {
    fn price(&self) -> f64 {
        self.pool.reserve_y as f64 / self.pool.reserve_x as f64
    }
    fn fee(&self) -> f64 {
        f64::from(self.fee_ppm) / 1e6
    }
    fn swap(&mut self, a_to_b: bool, amount: u64) -> Option<u64> {
        let side = if a_to_b { cpmm::Side::X } else { cpmm::Side::Y };
        self.pool
            .swap_exact_in(side, amount, 0)
            .ok()
            .map(|s| s.amount_out)
    }
    fn lp_exit(&self) -> ((u64, u64), (u64, u64)) {
        let mut p = self.pool;
        let out = p.withdraw(self.shares).expect("full exit");
        // LP fees are inside the reserves; they are reported as zero here and
        // show up in the PnL instead.
        let _ = self.initial;
        (out, (0, 0))
    }
}

/// Largest input that keeps the post-trade price on the profitable side of
/// `target`, found by doubling then bisection on a copy of the venue.
fn arb_amount<V: Venue>(v: &V, a_to_b: bool, target: f64) -> u64 {
    let ok = |amount: u64| -> bool {
        let mut c = v.clone();
        match c.swap(a_to_b, amount) {
            Some(_) if a_to_b => c.price() >= target,
            Some(_) => c.price() <= target,
            None => false,
        }
    };
    let mut lo = 0u64;
    let mut hi = 1u64 << 20;
    while ok(hi) {
        lo = hi;
        if hi > u64::MAX / 4 {
            break;
        }
        hi *= 2;
    }
    for _ in 0..40 {
        if hi - lo <= 1 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        if ok(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

#[derive(Default, Clone, Copy)]
struct Outcome {
    lp_pnl_vs_hodl_pct: f64,
    lp_fee_pct: f64,
    trader_cost_bps: f64,
    trader_volume_usd: f64,
    arb_volume_usd: f64,
    rejected: u64,
    out_of_range_steps: u64,
}

#[derive(Clone, Copy)]
struct Scenario {
    /// Split every order evenly across all venues instead of the best one.
    split: bool,
    steps: u32,
    sigma: f64,
    trades_per_step: u32,
    median_trade_usd: f64,
}

fn run<V: Venue>(
    mut venues: Vec<V>,
    deposits: Vec<(u64, u64)>,
    sc: &Scenario,
    seed: u64,
) -> Outcome {
    let mut rng = Rng(seed);
    let mut ext = P0;
    let mut o = Outcome::default();
    let (mut cost, mut vol) = (0.0, 0.0);
    for step in 0..sc.steps {
        ext *= (sc.sigma * rng.normal() - sc.sigma * sc.sigma / 2.0).exp();
        let mut slot = 1_000_000 + u64::from(step) * SLOTS_PER_STEP;
        for v in venues.iter_mut() {
            v.set_slot(slot);
            let (p, f) = (v.price(), v.fee());
            if p < ext * (1.0 - f) {
                let amount = arb_amount(v, false, ext * (1.0 - f));
                if amount > 0 && v.swap(false, amount).is_some() {
                    o.arb_volume_usd += amount as f64 / USDC;
                }
            } else if p > ext / (1.0 - f) {
                let amount = arb_amount(v, true, ext / (1.0 - f));
                if amount > 0 && v.swap(true, amount).is_some() {
                    o.arb_volume_usd += amount as f64 * ext / USDC;
                }
            }
            let gap = (v.price() / ext).ln().abs();
            if gap > 2.0 * f + 0.002 {
                o.out_of_range_steps += 1;
            }
        }
        for _ in 0..sc.trades_per_step {
            slot += 3;
            for v in venues.iter_mut() {
                v.set_slot(slot);
            }
            let usd = sc.median_trade_usd * (rng.normal() * 1.0).exp();
            let a_to_b = rng.next().is_multiple_of(2);
            let amount = if a_to_b {
                (usd * USDC / ext) as u64
            } else {
                (usd * USDC) as u64
            };
            if sc.split && venues.len() > 1 {
                let n = venues.len() as u64;
                let part = amount / n;
                let quotes: Option<Vec<u64>> =
                    venues.iter().map(|v| v.quote(a_to_b, part)).collect();
                if let Some(q) = quotes {
                    for v in venues.iter_mut() {
                        v.swap(a_to_b, part).unwrap();
                    }
                    let out: u64 = q.iter().sum();
                    let used = part * n;
                    let (vin, vout) = if a_to_b {
                        (used as f64 * ext, out as f64)
                    } else {
                        (used as f64, out as f64 * ext)
                    };
                    cost += vin - vout;
                    vol += vin;
                    continue;
                }
            }
            // Route to the best quote, as an aggregator would.
            let best = venues
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.quote(a_to_b, amount).map(|q| (i, q)))
                .max_by_key(|&(_, q)| q);
            let Some((i, _)) = best else {
                o.rejected += 1;
                continue;
            };
            let out = venues[i].swap(a_to_b, amount).unwrap();
            let (vin, vout) = if a_to_b {
                (amount as f64 * ext, out as f64)
            } else {
                (amount as f64, out as f64 * ext)
            };
            cost += vin - vout;
            vol += vin;
        }
    }
    let (mut end, mut hodl, mut start, mut fees) = (0.0, 0.0, 0.0, 0.0);
    for (v, (a0, b0)) in venues.iter().zip(&deposits) {
        let ((a, b), (fa, fb)) = v.lp_exit();
        end += a as f64 * ext + b as f64;
        fees += fa as f64 * ext + fb as f64;
        hodl += *a0 as f64 * ext + *b0 as f64;
        start += *a0 as f64 * P0 + *b0 as f64;
    }
    o.lp_pnl_vs_hodl_pct = 100.0 * (end - hodl) / start;
    o.lp_fee_pct = 100.0 * fees / start;
    o.trader_cost_bps = 1e4 * cost / vol;
    o.trader_volume_usd = vol / USDC;
    o
}

#[derive(Clone, Copy)]
enum Design {
    Cpmm,
    Clmm {
        spacing: u16,
        width: f64,
        shards: usize,
        split: bool,
        /// Dynamic fee strength in ppm of fee per 1% of price movement;
        /// 0 = fixed fee.
        dynamic_per_pct: u32,
    },
}

fn label(d: Design) -> String {
    match d {
        Design::Cpmm => "CPMM (baseline)".into(),
        Design::Clmm {
            spacing,
            width,
            shards,
            split,
            dynamic_per_pct,
        } => format!(
            "CLMM s={spacing} ±{:.0}% {}{}",
            width * 100.0,
            match (shards, split) {
                (1, _) => "1 shard".to_string(),
                (n, false) => format!("{n} shards, best-shard routing"),
                (n, true) => format!("{n} shards, order split across all"),
            },
            match dynamic_per_pct {
                0 => String::new(),
                k => format!(", dynamic +{:.2}% fee per 1% move", f64::from(k) / 1e4),
            }
        ),
    }
}

fn simulate(d: Design, fee_ppm: u32, sc: &Scenario, seed: u64) -> Outcome {
    let sc = &Scenario {
        split: matches!(d, Design::Clmm { split: true, .. }),
        ..*sc
    };
    match d {
        Design::Cpmm => {
            let (v, dep) = Cpmm::new(fee_ppm, CAPITAL_USD);
            run(vec![v], vec![dep], sc, seed)
        }
        Design::Clmm {
            spacing,
            width,
            shards,
            dynamic_per_pct,
            ..
        } => {
            let dynamic = dynamic_params(fee_ppm, spacing, dynamic_per_pct);
            let (vs, deps): (Vec<_>, Vec<_>) = (0..shards)
                .map(|_| {
                    let (mut v, dep) =
                        Clmm::new(fee_ppm, spacing, width, CAPITAL_USD / shards as f64);
                    v.dynamic = dynamic;
                    (v, dep)
                })
                .unzip();
            run(vs, deps, sc, seed)
        }
    }
}

/// ADR-0005 parameters for the simulation: `per_pct` ppm of fee per 1% of
/// price movement (1% is about 100 ticks), up to 0.80%; an episode lasts
/// while swaps come within a minute, half of it carries over, and nothing
/// is left after ten quiet minutes. No load term.
fn dynamic_params(base: u32, spacing: u16, per_pct: u32) -> DynamicFeeParams {
    if per_pct == 0 {
        return DynamicFeeParams::disabled(base);
    }
    let p = DynamicFeeParams {
        max_fee_rate_ppm: sharded_amm::constants::MAX_DYNAMIC_FEE_RATE_PPM,
        vol_coeff_ppm: per_pct * u32::from(spacing) / 100,
        load_coeff_ppm: 0,
        max_vol_accumulator: 10_000_000,
        filter_slots: SLOTS_PER_STEP as u16,
        decay_slots: 10 * SLOTS_PER_STEP as u16,
        reduction_bps: 5_000,
    };
    p.validate(base).expect("valid dynamic fee params");
    p
}

fn mean_sd(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let v = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (m, v.sqrt())
}

fn env_or(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `--dynamic`: fixed versus dynamic fee (ADR-0005).
fn dynamic_report(seeds: u64, base: &Scenario) -> String {
    let mut out = String::from("# ADR-0005: fixed versus per-shard dynamic fee\n\n");
    out.push_str(&format!(
        "Generated by `cargo run --release -p amm-sim --bin eng09 -- --dynamic` with {seeds} seeds x {} one-minute steps, \
         {} uninformed trades per step (median ${}), LP capital ${} per design, protocol share 20%. \
         Same model and limits as [eng09-results.md](eng09-results.md). The dynamic fee adds the stated fee per 1% of \
         price movement within an episode, up to 0.80%; see `dynamic_params` in `crates/sim/src/main.rs`. \
         An order split evenly over 8 identical shards gives exactly the 1-shard numbers in this model \
         (see eng09-results.md), so only 1 shard is listed.\n\n",
        base.steps, base.trades_per_step, base.median_trade_usd, CAPITAL_USD
    ));
    out.push_str("| Volatility | Base fee | Design | LP PnL vs HODL % | LP fees % | Trader cost bps | Arb volume $ | Rejected trades |\n");
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    for (vol_label, mult) in [("4%/day", 1.0), ("16%/day", 4.0)] {
        let sc = Scenario {
            sigma: base.sigma * mult,
            ..*base
        };
        for fee in [500u32, 1_000, 3_000] {
            for (spacing, width, shards) in [(10u16, 0.05, 1usize), (60, 0.20, 1)] {
                for dynamic_per_pct in [0u32, 2_000, 5_000] {
                    let d = Design::Clmm {
                        spacing,
                        width,
                        shards,
                        split: shards > 1,
                        dynamic_per_pct,
                    };
                    let rs: Vec<Outcome> = (0..seeds)
                        .map(|s| simulate(d, fee, &sc, 0xe9_0000 + s))
                        .collect();
                    let col =
                        |f: fn(&Outcome) -> f64| mean_sd(&rs.iter().map(f).collect::<Vec<_>>());
                    let (pnl, pnl_sd) = col(|o| o.lp_pnl_vs_hodl_pct);
                    let (fees, _) = col(|o| o.lp_fee_pct);
                    let (cost, cost_sd) = col(|o| o.trader_cost_bps);
                    let (av, _) = col(|o| o.arb_volume_usd);
                    let rej: u64 = rs.iter().map(|o| o.rejected).sum();
                    let line = format!(
                        "| {vol_label} | {} bps | {} | {pnl:+.3} ± {pnl_sd:.3} | {fees:.3} | {cost:.2} ± {cost_sd:.2} | {av:.0} | {rej} |\n",
                        fee / 100,
                        label(d)
                    );
                    eprint!("{line}");
                    out.push_str(&line);
                }
            }
        }
    }
    out.push_str("\nTrader cost is what uninformed traders paid relative to the external price (fee plus price impact). ");
    out.push_str("LP PnL vs HODL includes fees and the loss to arbitrage. ± is one standard deviation across seeds.\n");
    out
}

fn main() {
    let seeds = env_or("SIM_SEEDS", 6);
    let sc = Scenario {
        split: false,
        steps: env_or("SIM_STEPS", 1440) as u32,
        // ~4%/day SOL volatility at one-minute steps.
        sigma: 0.04 / (1440f64).sqrt(),
        trades_per_step: 2,
        median_trade_usd: 2_000.0,
    };
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--dynamic") {
        args.remove(0);
        let out = dynamic_report(seeds, &sc);
        match args.first() {
            Some(path) => std::fs::write(path, out).expect("write results"),
            None => print!("{out}"),
        }
        return;
    }
    let designs = [
        Design::Cpmm,
        Design::Clmm {
            spacing: 10,
            width: 0.05,
            shards: 1,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 10,
            width: 0.20,
            shards: 1,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 60,
            width: 0.05,
            shards: 1,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 60,
            width: 0.20,
            shards: 1,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 10,
            width: 0.05,
            shards: 8,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 10,
            width: 0.05,
            shards: 8,
            split: true,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 60,
            width: 0.20,
            shards: 8,
            split: false,
            dynamic_per_pct: 0,
        },
        Design::Clmm {
            spacing: 60,
            width: 0.20,
            shards: 8,
            split: true,
            dynamic_per_pct: 0,
        },
    ];
    let mut out = String::new();
    out.push_str("# ENG-09 simulation results\n\n");
    out.push_str(&format!(
        "Generated by `cargo run --release -p amm-sim --bin eng09` with {seeds} seeds x {} one-minute steps \
         (sigma {:.4}% per step, {} uninformed trades per step, median ${}), LP capital ${} per design, \
         protocol share 20%. See the module comment in `crates/sim/src/main.rs` for what the model leaves out.\n\n",
        sc.steps,
        sc.sigma * 100.0,
        sc.trades_per_step,
        sc.median_trade_usd,
        CAPITAL_USD
    ));
    out.push_str("| Fee | Design | LP PnL vs HODL % | LP fees % | Trader cost bps | Trader volume $ | Arb volume $ | Rejected trades | Off-price venue-steps |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for fee in [500u32, 1_000, 3_000] {
        for d in designs {
            let rs: Vec<Outcome> = (0..seeds)
                .map(|s| simulate(d, fee, &sc, 0xe9_0000 + s))
                .collect();
            let col = |f: fn(&Outcome) -> f64| mean_sd(&rs.iter().map(f).collect::<Vec<_>>());
            let (pnl, pnl_sd) = col(|o| o.lp_pnl_vs_hodl_pct);
            let (fees, _) = col(|o| o.lp_fee_pct);
            let (cost, cost_sd) = col(|o| o.trader_cost_bps);
            let (tv, _) = col(|o| o.trader_volume_usd);
            let (av, _) = col(|o| o.arb_volume_usd);
            let rej: u64 = rs.iter().map(|o| o.rejected).sum();
            let off: u64 = rs.iter().map(|o| o.out_of_range_steps).sum();
            let fees_cell = if matches!(d, Design::Cpmm) {
                "in reserves".to_string()
            } else {
                format!("{fees:.3}")
            };
            let line = format!(
                "| {} bps | {} | {pnl:+.3} ± {pnl_sd:.3} | {fees_cell} | {cost:.2} ± {cost_sd:.2} | {tv:.0} | {av:.0} | {rej} | {off} |\n",
                fee / 100,
                label(d)
            );
            eprint!("{line}");
            out.push_str(&line);
        }
    }
    out.push_str("\nTrader cost is what uninformed traders paid relative to the external price (fee plus price impact). ");
    out.push_str("LP PnL vs HODL includes fees and the loss to arbitrage. ± is one standard deviation across seeds.\n");
    match args.first() {
        Some(path) => std::fs::write(path, out).expect("write results"),
        None => print!("{out}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_run_is_sane() {
        let sc = Scenario {
            split: false,
            steps: 60,
            sigma: 0.001,
            trades_per_step: 2,
            median_trade_usd: 2_000.0,
        };
        for d in [
            Design::Cpmm,
            Design::Clmm {
                spacing: 10,
                width: 0.05,
                shards: 1,
                split: false,
                dynamic_per_pct: 0,
            },
            Design::Clmm {
                spacing: 60,
                width: 0.20,
                shards: 8,
                split: false,
                dynamic_per_pct: 0,
            },
            Design::Clmm {
                spacing: 60,
                width: 0.20,
                shards: 8,
                split: true,
                dynamic_per_pct: 0,
            },
        ] {
            let o = simulate(d, 3_000, &sc, 1);
            assert!(o.trader_volume_usd > 0.0);
            // Traders always pay at least the fee net of rounding.
            assert!(o.trader_cost_bps > 25.0, "{}", o.trader_cost_bps);
            assert!(o.lp_pnl_vs_hodl_pct.is_finite());
        }
    }

    #[test]
    fn dynamic_fee_charges_more_after_movement() {
        let sc = Scenario {
            split: false,
            steps: 240,
            sigma: 0.004,
            trades_per_step: 2,
            median_trade_usd: 2_000.0,
        };
        let d = |dynamic_per_pct| Design::Clmm {
            spacing: 60,
            width: 0.20,
            shards: 1,
            split: false,
            dynamic_per_pct,
        };
        let fixed = simulate(d(0), 1_000, &sc, 7);
        let dynamic = simulate(d(5_000), 1_000, &sc, 7);
        assert!(dynamic.lp_fee_pct > fixed.lp_fee_pct);
        assert!(dynamic.trader_cost_bps > fixed.trader_cost_bps);
        assert!(dynamic.arb_volume_usd < fixed.arb_volume_usd);
    }
}
