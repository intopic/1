//! `eng09 --fees`: the launch fee schedule proposed in ADR-0004, on the
//! product as built. Every market has 8 shards with equal capital; every
//! uninformed order goes through the real router (`amm_router::plan`: best
//! of 2 random shards, split only when it pays), and after the orders of
//! each step the rebalancer bot runs one round (`amm_router::rebalance_round`,
//! four keys) when a rebalance pays more than its transaction. The protocol
//! takes 16% of every fee. Each case is also run as one pool holding the
//! same capital, so the cost of sharding is visible.
//!
//! Three markets: a stablecoin pair, SOL/USDC and a memecoin. They differ in
//! volatility, range width and order flow (`profiles`). Arbitrage against
//! the external price is the same as in the rest of ENG-09 (each shard on
//! its own, while profitable after the fee); it pays no gas, which favours
//! arbitrage. The bot pays `TX_COST_USD` per rebalance and only sends one
//! that pays more. Same limits as eng09-results.md: no MEV, no JIT, no
//! priority-fee market, one LP position per shard.

use super::*;
use amm_router::{apply_rebalance, plan, rebalance_round, Book, MarketCfg, RouterConfig};

const SHARE_BPS: u16 = 1_600;
/// What one rebalance costs the bot in USD (5 000 lamports at $150, plus a
/// priority fee), and so the profit floor it uses.
const TX_COST_USD: f64 = 0.01;
const SHARDS: usize = 8;
/// Agave 4.3.2 cost model (validator-bench.md): a block holds 100M cost
/// units and every writable account at most 40M.
const BLOCK_LIMIT: f64 = 100e6;
const ACCOUNT_LIMIT: f64 = 40e6;

/// Cost units of a swap transaction with `k` legs. One leg: 41.9k CU
/// measured on the validator (37.0M CU in the busiest slot / 883 swaps)
/// plus the signature and ~7 write locks, which together fill the 40M
/// account limit at the measured 883 swaps per slot (44.7k). Each extra leg
/// adds 34.9k CU (tx-budget.md) and 6 write locks. An approximation of the
/// cost model, checked against the validator only for one leg.
fn tx_cost(k: usize) -> f64 {
    44_700.0 + 36_700.0 * (k as f64 - 1.0)
}

#[derive(Clone)]
struct Shard {
    book: Book,
    pos: PositionState,
    m: MarketCfg,
    slot: u64,
}

impl Venue for Shard {
    fn price(&self) -> f64 {
        price_of_sqrt(self.book.pool.sqrt_price)
    }
    fn fee(&self) -> f64 {
        f64::from(self.book.fee_ppm(&self.m, self.slot)) / 1e6
    }
    fn set_slot(&mut self, slot: u64) {
        self.slot = slot;
    }
    fn swap(&mut self, a_to_b: bool, amount: u64) -> Option<u64> {
        self.book
            .apply(&self.m, self.slot, a_to_b, amount, 0)
            .ok()
            .map(|o| o.amount_out)
    }
    fn lp_exit(&self) -> ((u64, u64), (u64, u64)) {
        let (mut pool, mut arrays, mut pos) = (self.book.pool, self.book.arrays.clone(), self.pos);
        let mut refs: Vec<&mut TickArray> = arrays.iter_mut().collect();
        let l = pos.liquidity as i128;
        let (a, b) = modify_position(&mut pool, &mut refs, self.m.tick_spacing, &mut pos, -l)
            .expect("full exit");
        let fees = (pos.owed_a, pos.owed_b);
        ((a + fees.0, b + fees.1), fees)
    }
}

/// One fee setting: base fee, and the dynamic fee (ppm of fee per 1% of
/// price movement, 0 = fixed) with its ceiling.
#[derive(Clone, Copy)]
struct Fee {
    base_ppm: u32,
    dyn_per_pct: u32,
    dyn_max_ppm: u32,
}

impl Fee {
    fn label(&self) -> String {
        let base = f64::from(self.base_ppm) / 1e4;
        match self.dyn_per_pct {
            0 => format!("{base:.2}% fixed"),
            k => format!(
                "{base:.2}% + dynamic (+{:.2}% per 1% move, max {:.2}%)",
                f64::from(k) / 1e4,
                f64::from(self.dyn_max_ppm) / 1e4
            ),
        }
    }
}

struct Profile {
    name: &'static str,
    capital_usd: f64,
    sigma_per_day: f64,
    width: f64,
    spacing: u16,
    trades_per_step: u32,
    median_trade_usd: f64,
    fees: Vec<Fee>,
}

fn profiles() -> Vec<Profile> {
    let fixed = |base_ppm| Fee {
        base_ppm,
        dyn_per_pct: 0,
        dyn_max_ppm: 0,
    };
    let dynamic = |base_ppm, dyn_max_ppm| Fee {
        base_ppm,
        dyn_per_pct: 2_000,
        dyn_max_ppm,
    };
    vec![
        Profile {
            name: "Stablecoin (0.2%/day)",
            capital_usd: CAPITAL_USD,
            sigma_per_day: 0.002,
            width: 0.01,
            spacing: 10,
            trades_per_step: 3,
            median_trade_usd: 5_000.0,
            fees: vec![fixed(100), fixed(500)],
        },
        Profile {
            name: "SOL/USDC (4%/day)",
            capital_usd: CAPITAL_USD,
            sigma_per_day: 0.04,
            width: 0.20,
            spacing: 60,
            trades_per_step: 2,
            median_trade_usd: 2_000.0,
            fees: vec![fixed(500), fixed(1_000), fixed(3_000), dynamic(500, 3_000)],
        },
        // The same order flow on ten times the capital: each shard is as
        // deep as the whole $1M pool above.
        Profile {
            name: "SOL/USDC (4%/day), $10M",
            capital_usd: 10.0 * CAPITAL_USD,
            sigma_per_day: 0.04,
            width: 0.20,
            spacing: 60,
            trades_per_step: 2,
            median_trade_usd: 2_000.0,
            fees: vec![fixed(500), dynamic(500, 3_000)],
        },
        Profile {
            name: "Memecoin (20%/day)",
            capital_usd: CAPITAL_USD,
            sigma_per_day: 0.20,
            width: 0.50,
            spacing: 60,
            trades_per_step: 3,
            median_trade_usd: 1_000.0,
            fees: vec![fixed(3_000), fixed(10_000), dynamic(3_000, 8_000)],
        },
    ]
}

fn dyn_params(f: Fee, spacing: u16) -> DynamicFeeParams {
    if f.dyn_per_pct == 0 {
        return DynamicFeeParams::disabled(f.base_ppm);
    }
    let p = DynamicFeeParams {
        max_fee_rate_ppm: f.dyn_max_ppm,
        ..dynamic_params(f.base_ppm, spacing, f.dyn_per_pct)
    };
    p.validate(f.base_ppm).expect("valid dynamic fee params");
    p
}

#[derive(Default, Clone, Copy)]
struct FeeOutcome {
    trader_cost_bps: f64,
    trader_volume_usd: f64,
    lp_pnl_vs_hodl_pct: f64,
    lp_fee_pct: f64,
    protocol_usd: f64,
    bot_net_usd: f64,
    rebalances: f64,
    /// Widest gap between two shards after each step, mean and max, bps.
    gap_mean_bps: f64,
    gap_max_bps: f64,
    legs_per_order: f64,
    /// Orders per slot the cost model allows: per-account limit (what one
    /// market gets on mainnet, where the block is shared) and block limit.
    orders_per_slot_account: f64,
    orders_per_slot_block: f64,
    rejected: u64,
}

fn gap_bps(v: &[Shard]) -> f64 {
    let p = v.iter().map(Venue::price);
    let (lo, hi) = p.fold((f64::MAX, 0.0f64), |(lo, hi), x| (lo.min(x), hi.max(x)));
    (hi / lo - 1.0) * 1e4
}

fn run_fee(
    p: &Profile,
    f: Fee,
    shards: usize,
    rc: RouterConfig,
    steps: u32,
    seed: u64,
) -> FeeOutcome {
    let m = MarketCfg {
        base_fee_ppm: f.base_ppm,
        protocol_share_bps: SHARE_BPS,
        tick_spacing: p.spacing,
        dynamic: dyn_params(f, p.spacing),
    };
    let (mut venues, deposits): (Vec<Shard>, Vec<(u64, u64)>) = (0..shards)
        .map(|id| {
            let (c, dep) = Clmm::new(
                f.base_ppm,
                p.spacing,
                p.width,
                p.capital_usd / shards as f64,
            );
            let book = Book {
                id: id as u8,
                pool: c.pool,
                fee_state: DynamicFeeState::default(),
                arrays: c.arrays,
                pause_flags: 0,
            };
            (
                Shard {
                    book,
                    pos: c.position,
                    m,
                    slot: 0,
                },
                dep,
            )
        })
        .unzip();
    let sigma = p.sigma_per_day / 1440f64.sqrt();
    let mut rng = Rng(seed);
    let mut ext = P0;
    let mut o = FeeOutcome::default();
    let (mut cost, mut vol, mut legs, mut orders) = (0.0, 0.0, 0usize, 0usize);
    let (mut gap_sum, mut bot) = (0.0, 0.0);
    let (mut load, mut total_cost) = (vec![0.0f64; shards], 0.0);
    for step in 0..steps {
        ext *= (sigma * rng.normal() - sigma * sigma / 2.0).exp();
        let mut slot = 1_000_000 + u64::from(step) * SLOTS_PER_STEP;
        for v in venues.iter_mut() {
            v.set_slot(slot);
            let (pr, fe) = (v.price(), v.fee());
            if pr < ext * (1.0 - fe) {
                let amount = arb_amount(v, false, ext * (1.0 - fe));
                if amount > 0 {
                    v.swap(false, amount);
                }
            } else if pr > ext / (1.0 - fe) {
                let amount = arb_amount(v, true, ext / (1.0 - fe));
                if amount > 0 {
                    v.swap(true, amount);
                }
            }
        }
        for _ in 0..p.trades_per_step {
            slot += 3;
            for v in venues.iter_mut() {
                v.set_slot(slot);
            }
            let usd = p.median_trade_usd * rng.normal().exp();
            let a_to_b = rng.next().is_multiple_of(2);
            let amount = if a_to_b {
                (usd * USDC / ext) as u64
            } else {
                (usd * USDC) as u64
            };
            let books: Vec<Book> = venues.iter().map(|v| v.book.clone()).collect();
            let Some(pl) = plan(&books, &m, slot, a_to_b, amount, rc, rng.next()) else {
                o.rejected += 1;
                continue;
            };
            let mut out = 0u64;
            for leg in &pl.legs {
                let got = venues[leg.id as usize]
                    .swap(a_to_b, leg.amount_in)
                    .expect("planned leg executes");
                assert_eq!(got, leg.amount_out, "the plan is what the program does");
                out += got;
            }
            let used: u64 = pl.legs.iter().map(|l| l.amount_in).sum();
            let (vin, vout) = if a_to_b {
                (used as f64 * ext, out as f64)
            } else {
                (used as f64, out as f64 * ext)
            };
            cost += vin - vout;
            vol += vin;
            legs += pl.legs.len();
            orders += 1;
            // A transaction's full cost is charged to every shard it writes.
            let c = tx_cost(pl.legs.len());
            total_cost += c;
            for leg in &pl.legs {
                load[leg.id as usize] += c;
            }
        }
        if shards > 1 {
            let mut books: Vec<Book> = venues.iter().map(|v| v.book.clone()).collect();
            let floor_b = (TX_COST_USD * USDC) as u64;
            let floor_a = (TX_COST_USD * USDC / ext) as u64;
            for r in rebalance_round(&books.clone(), &m, slot, [floor_a, floor_b], 4) {
                let profit = apply_rebalance(&mut books, &m, slot, &r).expect("rebalance");
                let usd = if r.input_is_a {
                    profit as f64 * ext
                } else {
                    profit as f64
                } / USDC;
                bot += usd - TX_COST_USD;
                o.rebalances += 1.0;
            }
            for (v, b) in venues.iter_mut().zip(books) {
                v.book = b;
            }
            let g = gap_bps(&venues);
            gap_sum += g;
            o.gap_max_bps = o.gap_max_bps.max(g);
        }
    }
    let (mut end, mut hodl, mut start, mut fees, mut proto) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (v, (a0, b0)) in venues.iter().zip(&deposits) {
        let ((a, b), (fa, fb)) = v.lp_exit();
        end += a as f64 * ext + b as f64;
        fees += fa as f64 * ext + fb as f64;
        hodl += *a0 as f64 * ext + *b0 as f64;
        start += *a0 as f64 * P0 + *b0 as f64;
        proto += v.book.pool.protocol_a as f64 * ext + v.book.pool.protocol_b as f64;
    }
    o.lp_pnl_vs_hodl_pct = 100.0 * (end - hodl) / start;
    o.lp_fee_pct = 100.0 * fees / start;
    o.protocol_usd = proto / USDC;
    o.bot_net_usd = bot;
    o.trader_cost_bps = 1e4 * cost / vol.max(1.0);
    o.trader_volume_usd = vol / USDC;
    o.legs_per_order = legs as f64 / orders.max(1) as f64;
    o.gap_mean_bps = gap_sum / f64::from(steps);
    let n = orders.max(1) as f64;
    let busiest = load.iter().copied().fold(0.0, f64::max) / n;
    o.orders_per_slot_account = ACCOUNT_LIMIT / busiest.max(1.0);
    o.orders_per_slot_block = BLOCK_LIMIT / (total_cost / n).max(1.0);
    o
}

pub fn fee_report(seeds: u64, steps: u32) -> String {
    let mut out = String::from("# Fee schedule: 8 shards with router and rebalancer\n\n");
    out.push_str(&format!(
        "Generated by `cargo run --release -p amm-sim --bin eng09 -- --fees` with {seeds} seeds x {steps} one-minute steps \
         (one day), LP capital ${CAPITAL_USD} per market unless the market says otherwise, protocol share {}%. Orders are routed by `amm_router::plan` \
         (best of 2 random shards, split only when it pays); after each minute the bot runs `amm_router::rebalance_round` \
         (4 keys) and pays ${TX_COST_USD} per rebalance. Model and limits: module comment of `crates/sim/src/fees.rs`.\n\n",
        SHARE_BPS / 100
    ));
    out.push_str("| Market | Fee | Design | Trader cost bps | Volume $/day | LP PnL vs HODL %/day | LP fees %/day | Protocol $/day | Bot net $/day | Rebalances/day | Shard gap bps (mean / max) | Legs per order | Orders/slot, account limit (vs 1 pool) | Orders/slot, block limit | Rejected |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for p in profiles() {
        for f in &p.fees {
            // The router with a 10 bps tax per extra shard (the default
            // until 2026-10-29), the default 3 bps, and no tax: splits
            // whenever the price improves at all (more shards per order,
            // fewer orders in parallel; docs/tx-budget.md, audit P-02).
            let low_tax = RouterConfig {
                split_tax_bps: 3,
                ..RouterConfig::default()
            };
            let high_tax = RouterConfig {
                split_tax_bps: 10,
                ..RouterConfig::default()
            };
            let price_first = RouterConfig {
                split_tax_bps: 0,
                min_gain_bps: 0,
                ..RouterConfig::default()
            };
            for (shards, rc, design) in [
                (1usize, RouterConfig::default(), "1 pool, same capital"),
                (SHARDS, high_tax, "8 shards + router (tax 10 bps) + bot"),
                (SHARDS, low_tax, "8 shards + router (tax 3 bps) + bot"),
                (SHARDS, price_first, "8 shards + router (no tax) + bot"),
            ] {
                let rs: Vec<FeeOutcome> = (0..seeds)
                    .map(|s| run_fee(&p, *f, shards, rc, steps, 0xfee_0000 + s))
                    .collect();
                let col =
                    |g: fn(&FeeOutcome) -> f64| mean_sd(&rs.iter().map(g).collect::<Vec<_>>());
                let (cost, cost_sd) = col(|o| o.trader_cost_bps);
                let (tv, _) = col(|o| o.trader_volume_usd);
                let (pnl, pnl_sd) = col(|o| o.lp_pnl_vs_hodl_pct);
                let (lpf, _) = col(|o| o.lp_fee_pct);
                let (proto, _) = col(|o| o.protocol_usd);
                let (bot, _) = col(|o| o.bot_net_usd);
                let (reb, _) = col(|o| o.rebalances);
                let (gm, _) = col(|o| o.gap_mean_bps);
                let gx = rs.iter().map(|o| o.gap_max_bps).fold(0.0, f64::max);
                let (lg, _) = col(|o| o.legs_per_order);
                let (acc, _) = col(|o| o.orders_per_slot_account);
                let (blk, _) = col(|o| o.orders_per_slot_block);
                let one = ACCOUNT_LIMIT / tx_cost(1);
                let rej: u64 = rs.iter().map(|o| o.rejected).sum();
                let (bot_cell, reb_cell, gap_cell) = if shards == 1 {
                    ("—".to_string(), "—".to_string(), "—".to_string())
                } else {
                    (
                        format!("{bot:.0}"),
                        format!("{reb:.0}"),
                        format!("{gm:.1} / {gx:.1}"),
                    )
                };
                let line = format!(
                    "| {} | {} | {design} | {cost:.2} ± {cost_sd:.2} | {tv:.0} | {pnl:+.3} ± {pnl_sd:.3} | {lpf:.3} | {proto:.0} | {bot_cell} | {reb_cell} | {gap_cell} | {lg:.2} | {acc:.0} ({:.2}×) | {blk:.0} | {rej} |\n",
                    p.name,
                    f.label(),
                    acc / one
                );
                eprint!("{line}");
                out.push_str(&line);
            }
        }
    }
    out.push_str("\nTrader cost: what uninformed traders paid against the external price (fee plus price impact). ");
    out.push_str("LP PnL vs HODL includes fees and the loss to arbitrage. Protocol: its 16% of all fees (traders, arbitrage, rebalances). ");
    out.push_str("Bot net: rebalance profit minus its transaction cost. ± is one standard deviation across seeds.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_fee_runs_are_sane() {
        let p = &profiles()[1];
        for f in &p.fees {
            let rc = RouterConfig::default();
            let one = run_fee(p, *f, 1, rc, 30, 3);
            let eight = run_fee(p, *f, SHARDS, rc, 30, 3);
            for o in [one, eight] {
                assert!(o.trader_volume_usd > 0.0);
                // Traders pay at least the base fee net of rounding.
                assert!(
                    o.trader_cost_bps > f64::from(f.base_ppm) / 100.0 * 0.9,
                    "{}",
                    o.trader_cost_bps
                );
                assert!(o.protocol_usd > 0.0);
            }
            // The bot never sends a rebalance that loses money.
            assert!(eight.bot_net_usd >= 0.0);
        }
    }
}
