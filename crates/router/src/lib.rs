//! Off-chain router for the sharded AMM (ADR-0007).
//!
//! Everything here runs the program's own engine (`sharded_amm::engine`) on
//! copies of the on-chain accounts, so a quote is exactly what the program
//! will do in the same slot with the same state.
//!
//! - [`plan`]: threshold routing. A small order goes to one shard, chosen by
//!   power of two choices so clients with the same snapshot do not all pick
//!   the same shard (parallelism audit P-01); a larger one is split evenly
//!   over `k` shards, where `k` grows only while each extra shard improves the
//!   output by at least `min_gain_bps + split_tax_bps`, because every shard a
//!   transaction writes is charged its full cost (P-02). Fewer shards per order means fewer write locks, so more
//!   orders can run in parallel (A-10), while large orders still get the
//!   price quality of a split (ENG-09).
//! - [`find_rebalance`]: the most profitable `rebalance_pair` between the
//!   cheapest and the dearest shard (ADR-0006), for any keeper or bot;
//!   [`rebalance_round`]: several of them on disjoint pairs, one per
//!   rebalancer key, for the bot (`tests/svm/examples/rebalancer.rs`).
//!
//! - [`recovery`]: what to do when a planned swap fails, from the program's
//!   error: refresh the snapshot, skip the shard, spread wider, or stop.
//!
//! Limits: the split is even across the chosen shards, which is optimal when
//! they hold the same liquidity at the same price (what rebalancing keeps
//! them close to) and approximate otherwise.

#[cfg(test)]
mod fee_manipulation;
pub mod recovery;

use sharded_amm::constants::{MAX_SWAP_TICK_ARRAYS, PAUSE_REBALANCE, PAUSE_SWAPS};
use sharded_amm::engine::{
    dynamic_fee, rebalance, swap, Pool, RebalanceParams, SwapOutcome, SwapParams,
};
use sharded_amm::errors::AmmError;
use sharded_amm::state::{
    array_span, array_start_for, DynamicFeeParams, DynamicFeeState, MarketConfig, ShardState,
    TickArray,
};

/// Accounts of a split swap before the per-shard ones: payer, market,
/// registry, user token in/out, token program, AMM program; plus the
/// ComputeBudget program (docs/tx-budget.md).
pub const TX_FIXED_ACCOUNTS: usize = 8;
/// Account locks per transaction (docs/tx-budget.md).
pub const TX_ACCOUNT_LIMIT: usize = 64;

/// The market fields a quote needs.
#[derive(Clone, Copy, Debug)]
pub struct MarketCfg {
    pub base_fee_ppm: u32,
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
    pub dynamic: DynamicFeeParams,
}

impl MarketCfg {
    pub fn from_config(c: &MarketConfig) -> Self {
        Self {
            base_fee_ppm: c.params.fee_rate_ppm,
            protocol_share_bps: c.params.protocol_share_bps,
            tick_spacing: c.params.tick_spacing,
            dynamic: c.dynamic_fees,
        }
    }
}

/// One shard: its state and the tick arrays the router knows about.
#[derive(Clone, Debug)]
pub struct Book {
    pub id: u8,
    pub pool: Pool,
    pub fee_state: DynamicFeeState,
    pub arrays: Vec<TickArray>,
    /// The shard's own pauses (`PAUSE_*`): a swap-paused shard quotes
    /// nothing (but can still be rebalanced against an open one), a
    /// rebalance-paused one is left out of `find_rebalance`.
    pub pause_flags: u8,
}

impl Book {
    pub fn from_state(id: u8, s: &ShardState, arrays: Vec<TickArray>) -> Result<Self, AmmError> {
        Ok(Self {
            id,
            pool: Pool::load(s)?,
            fee_state: s.dynamic_fee,
            arrays,
            pause_flags: s.pause_flags,
        })
    }

    /// Start ticks of the arrays a swap from the current tick passes, in
    /// order, up to `MAX_SWAP_TICK_ARRAYS`; only arrays the book holds.
    pub fn array_starts(&self, spacing: u16, a_to_b: bool) -> Vec<i32> {
        let span = array_span(spacing);
        let mut start = array_start_for(self.pool.tick, spacing);
        let mut out = Vec::new();
        while out.len() < MAX_SWAP_TICK_ARRAYS
            && self.arrays.iter().any(|a| a.start_tick_index == start)
        {
            out.push(start);
            start += if a_to_b { -span } else { span };
        }
        out
    }

    fn leg_arrays(&self, spacing: u16, a_to_b: bool) -> Vec<TickArray> {
        self.array_starts(spacing, a_to_b)
            .iter()
            .map(|s| {
                *self
                    .arrays
                    .iter()
                    .find(|a| a.start_tick_index == *s)
                    .unwrap()
            })
            .collect()
    }

    /// The fee rate a swap starting now would pay.
    pub fn fee_ppm(&self, m: &MarketCfg, slot: u64) -> u32 {
        dynamic_fee::quote_fee(
            &self.fee_state,
            &m.dynamic,
            m.base_fee_ppm,
            m.tick_spacing,
            self.pool.tick,
            slot,
        )
    }

    /// What `swap_exact_in` would return, without changing the book.
    pub fn quote(
        &self,
        m: &MarketCfg,
        slot: u64,
        a_to_b: bool,
        amount_in: u64,
        limit: u128,
    ) -> Result<SwapOutcome, AmmError> {
        self.clone().apply(m, slot, a_to_b, amount_in, limit)
    }

    /// Executes the swap on the book, as the program would.
    pub fn apply(
        &mut self,
        m: &MarketCfg,
        slot: u64,
        a_to_b: bool,
        amount_in: u64,
        limit: u128,
    ) -> Result<SwapOutcome, AmmError> {
        if self.pause_flags & PAUSE_SWAPS != 0 {
            return Err(AmmError::ShardPaused);
        }
        let mut copies = self.leg_arrays(m.tick_spacing, a_to_b);
        let mut refs: Vec<&mut TickArray> = copies.iter_mut().collect();
        let mut pool = self.pool;
        let mut fs = self.fee_state;
        let fee = dynamic_fee::begin(
            &mut fs,
            &m.dynamic,
            m.base_fee_ppm,
            m.tick_spacing,
            pool.tick,
            slot,
        );
        let out = swap(
            &mut pool,
            &mut refs,
            SwapParams {
                a_to_b,
                amount_in,
                sqrt_price_limit: limit,
                fee_rate_ppm: fee,
                protocol_share_bps: m.protocol_share_bps,
                tick_spacing: m.tick_spacing,
            },
        )?;
        dynamic_fee::finish(&mut fs, &m.dynamic, m.tick_spacing, pool.tick, slot);
        self.store(pool, fs, copies);
        Ok(out)
    }

    fn store(&mut self, pool: Pool, fs: DynamicFeeState, copies: Vec<TickArray>) {
        self.pool = pool;
        self.fee_state = fs;
        for c in copies {
            if let Some(a) = self
                .arrays
                .iter_mut()
                .find(|a| a.start_tick_index == c.start_tick_index)
            {
                *a = c;
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RouterConfig {
    /// At most this many shards per order.
    pub max_shards: usize,
    /// Each extra shard must raise the output by at least this much ...
    pub min_gain_bps: u32,
    /// ... plus this "parallelism tax" per extra shard. A transaction's full
    /// cost is charged to every shard it writes (audit P-02), so a split
    /// into k shards uses k times the per-shard capacity.
    pub split_tax_bps: u32,
    /// Absolute cost of one extra leg in output-token units (its CU times
    /// the priority fee, priced by the caller); 0 to ignore.
    pub leg_cost_out: u64,
    /// Power of two choices (audit P-01): the first shard is the better of
    /// `choices` shards drawn at random, weighted by active liquidity, so
    /// clients reading the same snapshot spread out instead of herding.
    /// 0 means always the global best (deterministic).
    pub choices: usize,
    /// ... unless the global best beats that pick by more than this.
    pub max_regret_bps: u32,
}

/// Split tax when the shards are idle, and when they are congested.
pub const QUIET_SPLIT_TAX_BPS: u32 = 3;
pub const BUSY_SPLIT_TAX_BPS: u32 = 10;
/// Priority fee (micro-lamports per CU) on the shard accounts at or below
/// which they count as idle, and at or above which as congested.
pub const QUIET_PRIORITY_FEE: u64 = 1_000;
pub const BUSY_PRIORITY_FEE: u64 = 100_000;

impl RouterConfig {
    /// The default config with the split tax set from how congested the
    /// shards are, measured by the recent priority fee paid to write them
    /// (e.g. the median of `getRecentPrioritizationFees` over the shard
    /// accounts). Parallelism only matters when the shards are full: idle,
    /// an order may split for the best price (3 bps tax); congested, each
    /// extra shard it writes costs other orders their slot, so the tax rises
    /// to 10 bps, linearly in the fee's logarithm between the two marks.
    /// gerfa, 2026-10-29 (fee simulation, docs/sim/fee-schedule-analysis.md).
    pub fn for_priority_fee(micro_lamports_per_cu: u64) -> Self {
        let f = micro_lamports_per_cu.clamp(QUIET_PRIORITY_FEE, BUSY_PRIORITY_FEE) as f64;
        let (q, b) = (QUIET_PRIORITY_FEE as f64, BUSY_PRIORITY_FEE as f64);
        let t = (f / q).ln() / (b / q).ln();
        let span = f64::from(BUSY_SPLIT_TAX_BPS - QUIET_SPLIT_TAX_BPS);
        Self {
            split_tax_bps: QUIET_SPLIT_TAX_BPS + (t * span).round() as u32,
            ..Self::default()
        }
    }
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            max_shards: 8,
            min_gain_bps: 2,
            split_tax_bps: QUIET_SPLIT_TAX_BPS,
            leg_cost_out: 0,
            choices: 2,
            max_regret_bps: 1,
        }
    }
}

/// splitmix64: a cheap, reproducible stream from a per-order seed such as
/// `hash(user, recent_blockhash)`.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Uniform in `0..n` (`n > 0`) from 128 random bits, by rejection, so a
/// total weight above 2^64 is drawn without bias (re-audit P2-3).
fn below(rng: &mut u64, n: u128) -> u128 {
    let zone = u128::MAX - (u128::MAX - n + 1) % n;
    loop {
        let x = (u128::from(next(rng)) << 64) | u128::from(next(rng));
        if x <= zone {
            return x % n;
        }
    }
}

/// `d` distinct indices of `books`, drawn with probability proportional to
/// active liquidity (uniform if all are empty). Weights are shifted right
/// together if their sum would not fit in 128 bits.
fn sample(books: &[Book], d: usize, seed: u64) -> Vec<usize> {
    let mut rng = seed;
    let mut left: Vec<usize> = (0..books.len()).collect();
    let mut out = Vec::with_capacity(d);
    while out.len() < d && !left.is_empty() {
        let mut shift = 0;
        let (w, total) = loop {
            let w: Vec<u128> = left
                .iter()
                .map(|&i| (books[i].pool.liquidity >> shift).max(1))
                .collect();
            match w.iter().try_fold(0u128, |s, x| s.checked_add(*x)) {
                Some(t) => break (w, t),
                None => shift += 8,
            }
        };
        let mut r = below(&mut rng, total);
        let mut pick = 0;
        for (j, wj) in w.iter().enumerate() {
            if r < *wj {
                pick = j;
                break;
            }
            r -= wj;
        }
        out.push(left.swap_remove(pick));
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leg {
    pub id: u8,
    pub amount_in: u64,
    pub amount_out: u64,
    /// Start ticks of the tick arrays to pass, in order.
    pub array_starts: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub legs: Vec<Leg>,
    pub amount_out: u64,
}

impl Plan {
    /// Accounts the transaction locks (docs/tx-budget.md).
    pub fn tx_accounts(&self) -> usize {
        TX_FIXED_ACCOUNTS
            + self
                .legs
                .iter()
                .map(|l| 4 + l.array_starts.len())
                .sum::<usize>()
    }
}

/// Splits `amount_in` evenly over `ids` and quotes every leg.
fn split(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    a_to_b: bool,
    amount_in: u64,
    ids: &[usize],
) -> Option<Plan> {
    let k = ids.len() as u64;
    let part = amount_in / k;
    let mut legs = Vec::with_capacity(ids.len());
    for (n, &i) in ids.iter().enumerate() {
        let amount = if n == 0 {
            amount_in - part * (k - 1)
        } else {
            part
        };
        if amount == 0 {
            return None;
        }
        let b = &books[i];
        let o = b.quote(m, slot, a_to_b, amount, 0).ok()?;
        legs.push(Leg {
            id: b.id,
            amount_in: amount,
            amount_out: o.amount_out,
            array_starts: b.array_starts(m.tick_spacing, a_to_b),
        });
    }
    let plan = Plan {
        amount_out: legs.iter().map(|l| l.amount_out).sum(),
        legs,
    };
    (plan.tx_accounts() <= TX_ACCOUNT_LIMIT).then_some(plan)
}

/// `out` is within `bps` of `base`.
fn within_regret(out: u64, base: u64, bps: u32) -> bool {
    u128::from(base) * 10_000 <= u128::from(out) * (10_000 + u128::from(bps))
}

/// Threshold routing: the smallest number of shards after which one more
/// shard would add less than `min_gain_bps + split_tax_bps` (and
/// `leg_cost_out`) to the output. The first shard is chosen by power of two
/// choices with `seed`; the others follow by quote.
///
/// The final plan's output for the full amount is never more than
/// `max_regret_bps` below the baseline: the better of the deterministic
/// threshold plan and the best single shard for the full amount
/// (re-audit P2-4); otherwise the baseline is returned.
pub fn plan(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    a_to_b: bool,
    amount_in: u64,
    cfg: RouterConfig,
    seed: u64,
) -> Option<Plan> {
    let picked = plan_from(books, m, slot, a_to_b, amount_in, cfg, seed);
    let base = baseline(books, m, slot, a_to_b, amount_in, cfg);
    match (picked, base) {
        (Some(p), Some(b)) if !within_regret(p.amount_out, b.amount_out, cfg.max_regret_bps) => {
            Some(b)
        }
        (p, b) => p.or(b),
    }
}

/// The better of the deterministic threshold plan and the best single shard
/// for the full amount.
fn baseline(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    a_to_b: bool,
    amount_in: u64,
    cfg: RouterConfig,
) -> Option<Plan> {
    let det = plan_from(
        books,
        m,
        slot,
        a_to_b,
        amount_in,
        RouterConfig { choices: 0, ..cfg },
        0,
    );
    let single = (0..books.len())
        .filter_map(|i| split(books, m, slot, a_to_b, amount_in, &[i]))
        .max_by_key(|p| p.amount_out);
    [det, single]
        .into_iter()
        .flatten()
        .max_by_key(|p| p.amount_out)
}

fn plan_from(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    a_to_b: bool,
    amount_in: u64,
    cfg: RouterConfig,
    seed: u64,
) -> Option<Plan> {
    if books.is_empty() || amount_in == 0 {
        return None;
    }
    let max = cfg.max_shards.clamp(1, books.len());
    // Rank shards by what a 1/max slice would get on each.
    let probe = (amount_in / max as u64).max(1);
    let quotes: Vec<Option<u64>> = books
        .iter()
        .map(|b| {
            b.quote(m, slot, a_to_b, probe, 0)
                .ok()
                .map(|o| o.amount_out)
        })
        .collect();
    let mut rank: Vec<(u64, usize)> = quotes
        .iter()
        .enumerate()
        .filter_map(|(i, q)| q.map(|q| (q, i)))
        .collect();
    rank.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
    let mut order: Vec<usize> = rank.iter().map(|r| r.1).collect();
    if order.is_empty() {
        return None;
    }
    if cfg.choices > 0 {
        // The better of the draws; on a tie the first draw, so ties stay
        // random rather than favouring low ids.
        let pick = sample(books, cfg.choices, seed)
            .into_iter()
            .filter_map(|i| quotes[i].map(|q| (q, i)))
            .fold(None, |acc: Option<(u64, usize)>, x| match acc {
                Some(a) if a.0 >= x.0 => Some(a),
                _ => Some(x),
            });
        if let Some((q, i)) = pick {
            if within_regret(q, rank[0].0, cfg.max_regret_bps) {
                order.retain(|&j| j != i);
                order.insert(0, i);
            }
        }
    }
    let mut best: Option<Plan> = None;
    for k in 1..=max.min(order.len()) {
        let Some(p) = split(books, m, slot, a_to_b, amount_in, &order[..k]) else {
            continue;
        };
        match &best {
            None => best = Some(p),
            Some(b) => {
                let bps = u128::from(10_000 + cfg.min_gain_bps + cfg.split_tax_bps);
                let need = u128::from(b.amount_out) * bps / 10_000 + u128::from(cfg.leg_cost_out);
                if u128::from(p.amount_out) >= need && p.amount_out > b.amount_out {
                    best = Some(p);
                } else {
                    break;
                }
            }
        }
    }
    best
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebalancePlan {
    pub lo: u8,
    pub hi: u8,
    pub input_is_a: bool,
    pub amount_in: u64,
    pub profit: u64,
    /// Arrays for `lo` (B -> A leg) and `hi` (A -> B leg).
    pub lo_array_starts: Vec<i32>,
    pub hi_array_starts: Vec<i32>,
}

fn rebalance_profit(
    lo: &Book,
    hi: &Book,
    m: &MarketCfg,
    slot: u64,
    input_is_a: bool,
    amount_in: u64,
) -> Option<u64> {
    let mut la = lo.leg_arrays(m.tick_spacing, false);
    let mut ha = hi.leg_arrays(m.tick_spacing, true);
    let mut lr: Vec<&mut TickArray> = la.iter_mut().collect();
    let mut hr: Vec<&mut TickArray> = ha.iter_mut().collect();
    let (mut lp, mut hp) = (lo.pool, hi.pool);
    rebalance(
        &mut lp,
        &mut lr,
        &mut hp,
        &mut hr,
        RebalanceParams {
            input_is_a,
            amount_in,
            min_profit: 0,
            fee_lo_ppm: lo.fee_ppm(m, slot),
            fee_hi_ppm: hi.fee_ppm(m, slot),
            protocol_share_bps: m.protocol_share_bps,
            tick_spacing: m.tick_spacing,
        },
    )
    .ok()
    .map(|o| o.profit)
}

/// The best `rebalance_pair` between the cheapest and the dearest shard, if
/// one pays at least `min_profit` in its input token.
pub fn find_rebalance(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    min_profit: u64,
) -> Option<RebalancePlan> {
    find_rebalance_min(books, m, slot, [min_profit, min_profit])
}

/// Like [`find_rebalance`], with its own floor per profit token:
/// `min_profit[0]` in A, `min_profit[1]` in B. A bot sets each to what a
/// transaction costs it in that token, so it never sends one that loses
/// money. When both directions clear their floor the one worth more is
/// taken, A valued in B at the geometric mean of the two shards' prices.
pub fn find_rebalance_min(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    min_profit: [u64; 2],
) -> Option<RebalancePlan> {
    // A swap-paused shard may still be rebalanced (recovery), but only
    // against a shard that is trading, as the program requires.
    let open = || {
        books
            .iter()
            .filter(|b| b.pause_flags & PAUSE_REBALANCE == 0)
    };
    let lo = open().min_by_key(|b| (b.pool.sqrt_price, b.id))?;
    let hi = open().max_by_key(|b| (b.pool.sqrt_price, u8::MAX - b.id))?;
    pair_plan(lo, hi, m, slot, min_profit).map(|(r, _)| r)
}

/// The best rebalance between `lo` and `hi` (`lo` cheaper), if one clears
/// its floor, with its value in B (A valued at the geometric mean of the
/// two prices).
fn pair_plan(
    lo: &Book,
    hi: &Book,
    m: &MarketCfg,
    slot: u64,
    min_profit: [u64; 2],
) -> Option<(RebalancePlan, f64)> {
    if lo.pause_flags & hi.pause_flags & PAUSE_SWAPS != 0 {
        return None;
    }
    if lo.pool.sqrt_price >= hi.pool.sqrt_price {
        return None;
    }
    let q64 = |x: u128| x as f64 / 18_446_744_073_709_551_616.0;
    // Cheap filter: a gap inside the two fees never pays (with a small
    // allowance for f64 rounding, so no paying pair is dropped).
    let ratio = (q64(hi.pool.sqrt_price) / q64(lo.pool.sqrt_price)).powi(2);
    let fees = f64::from(lo.fee_ppm(m, slot)) + f64::from(hi.fee_ppm(m, slot));
    if (ratio - 1.0) * 1e6 < fees * 0.99 {
        return None;
    }
    let b_per_a = q64(lo.pool.sqrt_price) * q64(hi.pool.sqrt_price);
    let mut best: Option<(u64, bool, u64, f64)> = None;
    for input_is_a in [false, true] {
        let floor = min_profit[usize::from(!input_is_a)].max(1);
        let Some((x, p)) = best_amount(lo, hi, m, slot, input_is_a) else {
            continue;
        };
        let value = if input_is_a {
            p as f64 * b_per_a
        } else {
            p as f64
        };
        if p >= floor && best.is_none_or(|(_, _, _, v)| value > v) {
            best = Some((x, input_is_a, p, value));
        }
    }
    let (amount_in, input_is_a, profit, value) = best?;
    Some((
        RebalancePlan {
            lo: lo.id,
            hi: hi.id,
            input_is_a,
            amount_in,
            profit,
            lo_array_starts: lo.array_starts(m.tick_spacing, false),
            hi_array_starts: hi.array_starts(m.tick_spacing, true),
        },
        value,
    ))
}

/// The input amount with the most profit in one direction, and that profit.
fn best_amount(
    lo: &Book,
    hi: &Book,
    m: &MarketCfg,
    slot: u64,
    input_is_a: bool,
) -> Option<(u64, u64)> {
    let f = |x: u64| rebalance_profit(lo, hi, m, slot, input_is_a, x).unwrap_or(0);
    // Coarse: powers of two; then a ternary search around the best.
    let (mut be, mut bp) = (0u32, 0u64);
    for e in 0..63u32 {
        let p = f(1u64 << e);
        if p > bp {
            (be, bp) = (e, p);
        }
    }
    if bp == 0 {
        return None;
    }
    let (mut a, mut b) = (1u64 << be.saturating_sub(1), (1u64 << (be + 1).min(62)));
    while b - a > 2 {
        let m1 = a + (b - a) / 3;
        let m2 = b - (b - a) / 3;
        if f(m1) < f(m2) {
            a = m1;
        } else {
            b = m2;
        }
    }
    (a..=b)
        .chain([1u64 << be])
        .map(|x| (x, f(x)))
        .max_by_key(|e| e.1)
}

/// One round for a bot with several rebalancer keys: up to `max_pairs`
/// rebalances on disjoint shard pairs with the largest total value
/// (a maximum weight matching of at most `max_pairs` edges). Every pair of
/// shards that can be rebalanced is priced (28 pairs with 8 shards; see
/// below for more shards), then
/// the best set of pairs with no shard in common is chosen exactly, by a
/// search over the pairs sorted by value that stops a branch as soon as it
/// cannot beat the best set found.
///
/// Disjoint pairs write disjoint accounts (each shard has its own state,
/// vaults and tick arrays), so the transactions can land in the same slot,
/// and each plan's profit is exact on the snapshot because no two plans
/// touch the same shard.
pub fn rebalance_round(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    min_profit: [u64; 2],
    max_pairs: usize,
) -> Vec<RebalancePlan> {
    let mut open: Vec<&Book> = books
        .iter()
        .filter(|b| b.pause_flags & PAUSE_REBALANCE == 0)
        .collect();
    // Above 4 * max_pairs shards (16 for four keys) only the 2 * max_pairs
    // cheapest and the 2 * max_pairs dearest are priced: at 56 shards all
    // 1 540 pairs take ~0.9 s on one core, too slow for a 200 ms slot. The
    // matching is exact over the shards kept.
    let keep = 2 * max_pairs;
    if open.len() > 2 * keep {
        open.sort_by_key(|b| (b.pool.sqrt_price, b.id));
        open.drain(keep..open.len() - keep);
    }
    let mut edges: Vec<(RebalancePlan, f64)> = Vec::new();
    for (i, x) in open.iter().enumerate() {
        for y in &open[i + 1..] {
            let (lo, hi) = if (x.pool.sqrt_price, x.id) < (y.pool.sqrt_price, y.id) {
                (x, y)
            } else {
                (y, x)
            };
            edges.extend(pair_plan(lo, hi, m, slot, min_profit));
        }
    }
    edges.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then((a.0.lo, a.0.hi).cmp(&(b.0.lo, b.0.hi)))
    });
    let mut best = (0.0, Vec::new());
    let mut cur = Vec::new();
    best_matching(&edges, 0, max_pairs, 0.0, &mut cur, &mut best);
    best.1.into_iter().map(|i| edges[i].0.clone()).collect()
}

/// Branch and bound over `edges` (sorted by value, highest first): the set
/// of at most `left` more disjoint edges, from `from` on, with the largest
/// total. A branch stops when even `left` copies of its next edge's value
/// cannot beat the best.
fn best_matching(
    edges: &[(RebalancePlan, f64)],
    from: usize,
    left: usize,
    total: f64,
    cur: &mut Vec<usize>,
    best: &mut (f64, Vec<usize>),
) {
    if total > best.0 {
        *best = (total, cur.clone());
    }
    if left == 0 {
        return;
    }
    for i in from..edges.len() {
        let (e, v) = &edges[i];
        if total + v * left as f64 <= best.0 {
            break;
        }
        let used = |id: u8| {
            cur.iter()
                .any(|&j| edges[j].0.lo == id || edges[j].0.hi == id)
        };
        if used(e.lo) || used(e.hi) {
            continue;
        }
        cur.push(i);
        best_matching(edges, i + 1, left - 1, total + v, cur, best);
        cur.pop();
    }
}

/// Executes a rebalance on the books, as the program would. Returns the
/// profit.
pub fn apply_rebalance(
    books: &mut [Book],
    m: &MarketCfg,
    slot: u64,
    r: &RebalancePlan,
) -> Result<u64, AmmError> {
    let li = books
        .iter()
        .position(|b| b.id == r.lo)
        .ok_or(AmmError::ShardMismatch)?;
    let hi = books
        .iter()
        .position(|b| b.id == r.hi)
        .ok_or(AmmError::ShardMismatch)?;
    let (lo_b, hi_b) = (&books[li], &books[hi]);
    let mut la = lo_b.leg_arrays(m.tick_spacing, false);
    let mut ha = hi_b.leg_arrays(m.tick_spacing, true);
    let (mut lp, mut hp) = (lo_b.pool, hi_b.pool);
    let (mut lf, mut hf) = (lo_b.fee_state, hi_b.fee_state);
    let fee = |fs: &mut DynamicFeeState, p: &Pool| {
        dynamic_fee::begin(fs, &m.dynamic, m.base_fee_ppm, m.tick_spacing, p.tick, slot)
    };
    let (fl, fh) = (fee(&mut lf, &lp), fee(&mut hf, &hp));
    let out = {
        let mut lr: Vec<&mut TickArray> = la.iter_mut().collect();
        let mut hr: Vec<&mut TickArray> = ha.iter_mut().collect();
        rebalance(
            &mut lp,
            &mut lr,
            &mut hp,
            &mut hr,
            RebalanceParams {
                input_is_a: r.input_is_a,
                amount_in: r.amount_in,
                min_profit: 0,
                fee_lo_ppm: fl,
                fee_hi_ppm: fh,
                protocol_share_bps: m.protocol_share_bps,
                tick_spacing: m.tick_spacing,
            },
        )?
    };
    dynamic_fee::finish(&mut lf, &m.dynamic, m.tick_spacing, lp.tick, slot);
    dynamic_fee::finish(&mut hf, &m.dynamic, m.tick_spacing, hp.tick, slot);
    books[li].store(lp, lf, la);
    books[hi].store(hp, hf, ha);
    Ok(out.profit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::prelude::Pubkey;
    use sharded_amm::engine::{modify_position, PositionState};
    use sharded_amm::math::tick_math::sqrt_price_at_tick;

    const SPACING: u16 = 60;

    pub(crate) fn cfg() -> MarketCfg {
        MarketCfg {
            base_fee_ppm: 1_000,
            protocol_share_bps: 2_000,
            tick_spacing: SPACING,
            dynamic: DynamicFeeParams::disabled(1_000),
        }
    }

    /// `n` identical shards at tick 0 with one position over +-6000 ticks.
    pub(crate) fn books(n: u8, liquidity: u128) -> Vec<Book> {
        let span = array_span(SPACING);
        (0..n)
            .map(|id| {
                let first = array_start_for(-6_000, SPACING);
                let mut arrays: Vec<TickArray> = (0..8)
                    .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
                    .collect();
                let mut pool = Pool::at_price(sqrt_price_at_tick(0).unwrap()).unwrap();
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
                let mut refs: Vec<&mut TickArray> = arrays.iter_mut().collect();
                modify_position(&mut pool, &mut refs, SPACING, &mut pos, liquidity as i128)
                    .unwrap();
                Book {
                    id,
                    pool,
                    fee_state: DynamicFeeState::default(),
                    arrays,
                    pause_flags: 0,
                }
            })
            .collect()
    }

    #[test]
    fn small_orders_use_one_shard_large_orders_split() {
        let bs = books(8, 1 << 40);
        let small = plan(&bs, &cfg(), 0, true, 1_000_000, RouterConfig::default(), 0).unwrap();
        assert_eq!(small.legs.len(), 1);
        let large = plan(&bs, &cfg(), 0, true, 1 << 38, RouterConfig::default(), 0).unwrap();
        assert!(large.legs.len() > 1, "{} legs", large.legs.len());
        assert!(large.legs.len() <= 8 && large.tx_accounts() <= TX_ACCOUNT_LIMIT);
        // Never worse than the best single shard.
        // (Here one shard cannot even fill it within three tick arrays.)
        let one = bs[0]
            .quote(&cfg(), 0, true, 1 << 38, 0)
            .map_or(0, |o| o.amount_out);
        assert!(large.amount_out >= one);
        // The plan's numbers are what executing the legs gives.
        let mut exec = bs.clone();
        let got: u64 = large
            .legs
            .iter()
            .map(|l| {
                let b = exec.iter_mut().find(|b| b.id == l.id).unwrap();
                let o = b.apply(&cfg(), 0, true, l.amount_in, 0).unwrap();
                assert_eq!(o.amount_out, l.amount_out);
                o.amount_out
            })
            .sum();
        assert_eq!(got, large.amount_out);
        assert_eq!(
            large.legs.iter().map(|l| l.amount_in).sum::<u64>(),
            1 << 38,
            "the whole input is routed"
        );
    }

    #[test]
    fn legs_grow_with_size_and_shrink_with_threshold() {
        let bs = books(8, 1 << 40);
        let legs = |amount: u64, min_gain_bps: u32| {
            plan(
                &bs,
                &cfg(),
                0,
                false,
                amount,
                RouterConfig {
                    min_gain_bps,
                    split_tax_bps: 0,
                    ..RouterConfig::default()
                },
                0,
            )
            .unwrap()
            .legs
            .len()
        };
        let sizes = [1u64 << 20, 1 << 30, 1 << 34, 1 << 36, 1 << 38];
        let counts: Vec<usize> = sizes.iter().map(|&a| legs(a, 2)).collect();
        assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{counts:?}");
        assert!(legs(1 << 38, 50) <= legs(1 << 38, 2));
        assert_eq!(legs(1 << 38, 0).max(1), legs(1 << 38, 0));
    }

    #[test]
    fn prefers_the_cheaper_shard_for_a_small_order() {
        let mut bs = books(4, 1 << 40);
        // Push shard 2's price down: A is cheaper there, so a buyer of A
        // (B -> A) should go to shard 2.
        bs[2].apply(&cfg(), 0, true, 1 << 36, 0).unwrap();
        let p = plan(&bs, &cfg(), 0, false, 1_000_000, RouterConfig::default(), 0).unwrap();
        assert_eq!(p.legs.len(), 1);
        assert_eq!(p.legs[0].id, 2);
    }

    #[test]
    fn paused_shards_are_left_out() {
        let mut bs = books(4, 1 << 40);
        bs[2].apply(&cfg(), 0, true, 1 << 36, 0).unwrap();
        bs[2].pause_flags = PAUSE_SWAPS;
        assert_eq!(
            bs[2].quote(&cfg(), 0, false, 1_000, 0),
            Err(AmmError::ShardPaused)
        );
        for seed in 0..200 {
            let p = plan(
                &bs,
                &cfg(),
                0,
                false,
                1 << 34,
                RouterConfig::default(),
                seed,
            )
            .unwrap();
            assert!(p.legs.iter().all(|l| l.id != 2), "seed {seed}");
        }
        // Shard 2 is the cheapest, but rebalancing is paused there.
        bs[2].pause_flags = PAUSE_REBALANCE;
        let r = find_rebalance(&bs, &cfg(), 0, 0);
        assert!(r.as_ref().is_none_or(|r| r.lo != 2 && r.hi != 2), "{r:?}");
        bs[2].pause_flags = 0;
        assert_eq!(find_rebalance(&bs, &cfg(), 0, 0).unwrap().lo, 2);
        // Swaps paused there: recovery rebalancing still brings it back.
        bs[2].pause_flags = PAUSE_SWAPS;
        let r = find_rebalance(&bs, &cfg(), 0, 0).unwrap();
        assert_eq!(r.lo, 2);
        let mut after = bs.clone();
        apply_rebalance(&mut after, &cfg(), 0, &r).unwrap();
        assert!(after[2].pool.sqrt_price > bs[2].pool.sqrt_price);
        // Not against another paused shard.
        for b in bs.iter_mut() {
            b.pause_flags = PAUSE_SWAPS;
        }
        assert_eq!(find_rebalance(&bs, &cfg(), 0, 0), None);
    }

    #[test]
    fn rebalance_search_closes_the_gap() {
        let mut bs = books(8, 1 << 40);
        bs[3].apply(&cfg(), 0, true, 1 << 37, 0).unwrap();
        let gap0 = bs.iter().map(|b| b.pool.sqrt_price).max().unwrap()
            - bs.iter().map(|b| b.pool.sqrt_price).min().unwrap();
        let r = find_rebalance(&bs, &cfg(), 0, 0).expect("profitable");
        assert_eq!(r.lo, 3);
        let profit = apply_rebalance(&mut bs, &cfg(), 0, &r).unwrap();
        assert_eq!(profit, r.profit);
        let gap1 = bs[r.hi as usize].pool.sqrt_price - bs[3].pool.sqrt_price;
        assert!(gap1 < gap0);
        // Near the optimum: 10% more or less pays no more.
        for x in [r.amount_in / 10 * 9, r.amount_in / 10 * 11] {
            let mut again = books(8, 1 << 40);
            again[3].apply(&cfg(), 0, true, 1 << 37, 0).unwrap();
            let p = rebalance_profit(&again[3], &again[r.hi as usize], &cfg(), 0, r.input_is_a, x)
                .unwrap_or(0);
            assert!(p <= r.profit, "{p} > {}", r.profit);
        }
        // Equal prices: nothing to do.
        assert_eq!(find_rebalance(&books(8, 1 << 40), &cfg(), 0, 0), None);
    }

    #[test]
    fn a_round_uses_disjoint_pairs_and_closes_every_gap() {
        // Four shards pushed down, four up: one round can fix all four gaps
        // at once, one pair per rebalancer key.
        let m = cfg();
        let mut bs = books(8, 1 << 40);
        for (i, b) in bs.iter_mut().enumerate() {
            b.apply(&m, 0, i % 2 == 0, (1 << 36) + (i as u64) * (1 << 32), 0)
                .unwrap();
        }
        let gap = |bs: &[Book]| {
            bs.iter().map(|b| b.pool.sqrt_price).max().unwrap()
                - bs.iter().map(|b| b.pool.sqrt_price).min().unwrap()
        };
        let gap0 = gap(&bs);
        let round = rebalance_round(&bs, &m, 0, [1, 1], 4);
        assert_eq!(round.len(), 4);
        let mut seen = Vec::new();
        for r in &round {
            assert!(!seen.contains(&r.lo) && !seen.contains(&r.hi), "{round:?}");
            seen.extend([r.lo, r.hi]);
        }
        // Applied one after the other on the same books, every plan pays
        // exactly what it was priced at: the pairs do not interact.
        for r in &round {
            assert_eq!(apply_rebalance(&mut bs, &m, 0, r).unwrap(), r.profit);
        }
        assert!(gap(&bs) * 4 < gap0, "{} vs {gap0}", gap(&bs));
        // No more than `max_pairs`, and nothing below the floor.
        assert!(rebalance_round(&bs, &m, 0, [1, 1], 0).is_empty());
        assert!(rebalance_round(&bs, &m, 0, [u64::MAX, u64::MAX], 4).is_empty());
    }

    /// Value in B of a set of plans, priced on `bs`.
    fn round_value(bs: &[Book], m: &MarketCfg, plans: &[RebalancePlan]) -> f64 {
        plans
            .iter()
            .map(|r| {
                let (lo, hi) = (&bs[r.lo as usize], &bs[r.hi as usize]);
                pair_plan(lo, hi, m, 0, [1, 1]).unwrap().1
            })
            .sum()
    }

    #[test]
    fn a_round_is_the_best_set_of_disjoint_pairs() {
        // The user's proposal: price all 28 pairs and take the best set of
        // up to 4 with no shard in common. Checked against every matching
        // (brute force), and against the earlier greedy round (widest gap
        // first), on random worlds with uneven depth and drifted prices.
        let m = cfg();
        let mut r = 11u64;
        let mut strictly_better = 0;
        let mut gain = Vec::new();
        for world in 0..40 {
            let mut bs: Vec<Book> = (0..8u8)
                .map(|id| {
                    let mut b = books(1, 1u128 << (36 + next(&mut r) % 6)).remove(0);
                    b.id = id;
                    b
                })
                .collect();
            for b in bs.iter_mut() {
                // Up to ~2% of price on the shallowest shard.
                let x = 1 + next(&mut r) % (1 << 31);
                b.apply(&m, 0, next(&mut r).is_multiple_of(2), x, 0)
                    .unwrap();
            }
            let round = rebalance_round(&bs, &m, 0, [1, 1], 4);
            let mut ids: Vec<u8> = round.iter().flat_map(|p| [p.lo, p.hi]).collect();
            let n = ids.len();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), n, "world {world}: pairs share a shard");
            let got = round_value(&bs, &m, &round);
            // Brute force over every matching of the 8 shards.
            let mut edges = Vec::new();
            for i in 0..8u8 {
                for j in i + 1..8 {
                    let (lo, hi) =
                        if bs[i as usize].pool.sqrt_price < bs[j as usize].pool.sqrt_price {
                            (i, j)
                        } else {
                            (j, i)
                        };
                    if let Some((_, v)) =
                        pair_plan(&bs[lo as usize], &bs[hi as usize], &m, 0, [1, 1])
                    {
                        edges.push((lo, hi, v));
                    }
                }
            }
            fn brute(e: &[(u8, u8, f64)], used: u8, k: usize) -> f64 {
                if k == 0 {
                    return 0.0;
                }
                let mut best = 0.0f64;
                for (i, &(a, b, v)) in e.iter().enumerate() {
                    if used & (1 << a) == 0 && used & (1 << b) == 0 {
                        best = best.max(v + brute(&e[i + 1..], used | 1 << a | 1 << b, k - 1));
                    }
                }
                best
            }
            let opt = brute(&edges, 0, 4);
            assert!(
                (got - opt).abs() <= opt * 1e-12,
                "world {world}: {got} vs {opt}"
            );
            // The greedy round it replaces (widest gap among the shards left).
            let mut left = bs.clone();
            let mut greedy = Vec::new();
            while greedy.len() < 4 {
                let Some(p) = find_rebalance_min(&left, &m, 0, [1, 1]) else {
                    break;
                };
                left.retain(|b| b.id != p.lo && b.id != p.hi);
                greedy.push(p);
            }
            let g = round_value(&bs, &m, &greedy);
            assert!(
                got >= g * (1.0 - 1e-12),
                "world {world}: {got} < greedy {g}"
            );
            if got > g * 1.01 {
                strictly_better += 1;
            }
            gain.push(if g > 0.0 { got / g } else { 1.0 });
            // Applied one after the other, every plan pays what it was priced at.
            let mut after = bs.clone();
            for p in &round {
                assert_eq!(apply_rebalance(&mut after, &m, 0, p).unwrap(), p.profit);
            }
        }
        gain.sort_by(f64::total_cmp);
        println!(
            "matching beat the greedy round by > 1% in {strictly_better} of 40 worlds; value / greedy: median {:.3}, max {:.3}",
            gain[gain.len() / 2],
            gain[gain.len() - 1]
        );
        assert!(strictly_better > 0, "no world where the matching helps");
    }

    #[test]
    fn each_direction_has_its_own_profit_floor() {
        let m = cfg();
        let mut bs = books(2, 1 << 40);
        bs[1].apply(&m, 0, false, 1 << 37, 0).unwrap();
        let free = find_rebalance_min(&bs, &m, 0, [1, 1]).unwrap();
        let (p, a) = (free.profit, free.input_is_a);
        // The winning direction below its floor: the other one, or nothing.
        let mut floor = [1, 1];
        floor[usize::from(!a)] = p + 1;
        if let Some(r) = find_rebalance_min(&bs, &m, 0, floor) {
            assert_ne!(r.input_is_a, a);
            assert!(r.profit >= floor[usize::from(!r.input_is_a)]);
        }
        assert!(find_rebalance_min(&bs, &m, 0, [u64::MAX, u64::MAX]).is_none());
    }

    /// First-leg shard of 1 000 small orders, one seed each, same snapshot.
    fn spread(bs: &[Book], rc: RouterConfig) -> Vec<usize> {
        let mut n = vec![0usize; bs.len()];
        for seed in 0..1_000u64 {
            let p = plan(bs, &cfg(), 0, seed % 2 == 0, 1_000_000, rc, seed).unwrap();
            assert_eq!(p.legs.len(), 1);
            n[p.legs[0].id as usize] += 1;
        }
        n
    }

    #[test]
    fn final_output_stays_within_the_regret_bound() {
        // Re-audit P2-4: the regret was checked on a probe of
        // amount_in / max_shards, then the full order could go to a shallower
        // shard (4.87 bps worse with max_regret_bps = 1). Heterogeneous depth
        // and drifted prices; the bound now holds on the final output.
        let m = cfg();
        let mut r = 7u64;
        for world in 0..30 {
            let mut bs: Vec<Book> = (0..8u8)
                .map(|id| {
                    let mut b = books(1, 1u128 << (34 + next(&mut r) % 10)).remove(0);
                    b.id = id;
                    b
                })
                .collect();
            for b in bs.iter_mut() {
                let x = 1 + next(&mut r) % (1 << 28);
                b.apply(&m, 0, next(&mut r).is_multiple_of(2), x, 0)
                    .unwrap();
            }
            for k in 0..40u64 {
                let a_to_b = next(&mut r).is_multiple_of(2);
                let amount = 1 + next(&mut r) % (1 << (12 + next(&mut r) % 20));
                let rc = RouterConfig::default();
                let Some(p) = plan(&bs, &m, 0, a_to_b, amount, rc, world * 100 + k) else {
                    continue;
                };
                let base = baseline(&bs, &m, 0, a_to_b, amount, rc).unwrap();
                assert!(
                    within_regret(p.amount_out, base.amount_out, rc.max_regret_bps),
                    "world {world} order {k}: {} vs {}",
                    p.amount_out,
                    base.amount_out
                );
                for i in 0..bs.len() {
                    if let Some(s) = split(&bs, &m, 0, a_to_b, amount, &[i]) {
                        assert!(within_regret(p.amount_out, s.amount_out, rc.max_regret_bps));
                    }
                }
            }
        }
    }

    #[test]
    fn sampling_is_fair_above_64_bits() {
        // Re-audit P2-3: eight equal shards with L = 2^65 all went to shard 0
        // (only the weights matter here, so the pools are set directly).
        let mut eq = books(8, 1 << 40);
        for b in eq.iter_mut() {
            b.pool.liquidity = 1 << 65;
        }
        let mut n = [0usize; 8];
        for seed in 0..8_000 {
            for i in sample(&eq, 2, seed) {
                n[i] += 1;
            }
        }
        assert!(n.iter().all(|&c| (1_700..=2_300).contains(&c)), "{n:?}");
        // Heterogeneous weights above 2^64: first draw ~ proportional to L.
        let mut bs = books(2, 1 << 40);
        bs[0].pool.liquidity = 1 << 66;
        bs[1].pool.liquidity = 3 << 66;
        let mut first = [0usize; 2];
        for seed in 0..4_000 {
            first[sample(&bs, 1, seed)[0]] += 1;
        }
        assert!((850..=1_150).contains(&first[0]), "{first:?}");
        // The order of the books does not change who is drawn, by id.
        let mut rev = bs.clone();
        rev.reverse();
        let mut by_id = [0usize; 2];
        for seed in 0..4_000 {
            by_id[rev[sample(&rev, 1, seed)[0]].id as usize] += 1;
        }
        assert!((850..=1_150).contains(&by_id[0]), "{by_id:?}");
        // Weights whose sum overflows u128 are scaled, not wrapped.
        let mut huge = books(3, 1 << 40);
        for b in huge.iter_mut() {
            b.pool.liquidity = u128::MAX / 2;
        }
        let mut hit = [0usize; 3];
        for seed in 0..3_000 {
            hit[sample(&huge, 1, seed)[0]] += 1;
        }
        assert!(hit.iter().all(|&c| (800..=1_200).contains(&c)), "{hit:?}");
    }

    #[test]
    fn same_snapshot_spreads_instead_of_herding() {
        let bs = books(8, 1 << 40);
        let herd = spread(
            &bs,
            RouterConfig {
                choices: 0,
                ..RouterConfig::default()
            },
        );
        assert_eq!(herd[0], 1_000, "deterministic: everyone on shard 0");
        let n = spread(&bs, RouterConfig::default());
        assert!(n.iter().all(|&c| (60..=200).contains(&c)), "{n:?}");
    }

    #[test]
    fn a_tiny_edge_does_not_herd_but_a_real_one_wins() {
        // A small sale of A makes shard 3 a hair cheaper for buyers of A.
        let mut bs = books(8, 1 << 40);
        bs[3].apply(&cfg(), 0, true, 1 << 24, 0).unwrap();
        let mut det = vec![0usize; 8];
        let mut p2 = vec![0usize; 8];
        for seed in 0..1_000u64 {
            let d = plan(
                &bs,
                &cfg(),
                0,
                false,
                1_000_000,
                RouterConfig {
                    choices: 0,
                    ..RouterConfig::default()
                },
                seed,
            )
            .unwrap();
            det[d.legs[0].id as usize] += 1;
            let r = plan(
                &bs,
                &cfg(),
                0,
                false,
                1_000_000,
                RouterConfig::default(),
                seed,
            )
            .unwrap();
            p2[r.legs[0].id as usize] += 1;
        }
        assert_eq!(det[3], 1_000, "{det:?}");
        // The hair-better shard wins every draw it is in (~2/8 of orders),
        // not all of them.
        assert!(p2.iter().all(|&c| (60..=300).contains(&c)), "{p2:?}");
        // A clear edge (well over 1 bp) is always taken.
        let mut bs = books(8, 1 << 40);
        bs[3].apply(&cfg(), 0, true, 1 << 33, 0).unwrap();
        for seed in 0..200u64 {
            let r = plan(
                &bs,
                &cfg(),
                0,
                false,
                1_000_000,
                RouterConfig::default(),
                seed,
            )
            .unwrap();
            assert_eq!(r.legs[0].id, 3);
        }
        // And a clearly worse shard is never the pick.
        let mut bs = books(8, 1 << 40);
        bs[5].apply(&cfg(), 0, false, 1 << 33, 0).unwrap();
        for seed in 0..1_000u64 {
            let r = plan(
                &bs,
                &cfg(),
                0,
                false,
                1_000_000,
                RouterConfig::default(),
                seed,
            )
            .unwrap();
            assert_ne!(r.legs[0].id, 5);
        }
    }

    #[test]
    fn split_tax_rises_with_congestion() {
        let tax = |f| RouterConfig::for_priority_fee(f).split_tax_bps;
        assert_eq!(tax(0), QUIET_SPLIT_TAX_BPS);
        assert_eq!(tax(QUIET_PRIORITY_FEE), QUIET_SPLIT_TAX_BPS);
        assert_eq!(tax(BUSY_PRIORITY_FEE), BUSY_SPLIT_TAX_BPS);
        assert_eq!(tax(u64::MAX), BUSY_SPLIT_TAX_BPS);
        assert_eq!(tax(10_000), 7, "halfway on a log scale");
        let mut last = 0;
        for f in (0..=200_000).step_by(500) {
            assert!(tax(f) >= last);
            last = tax(f);
        }
        assert_eq!(
            RouterConfig::for_priority_fee(0).split_tax_bps,
            RouterConfig::default().split_tax_bps
        );
    }

    #[test]
    fn split_tax_keeps_most_orders_on_one_or_two_shards() {
        let bs = books(8, 1 << 40);
        let legs = |amount: u64, rc: RouterConfig| {
            plan(&bs, &cfg(), 0, true, amount, rc, 7)
                .unwrap()
                .legs
                .len()
        };
        let free = RouterConfig {
            split_tax_bps: 0,
            ..RouterConfig::default()
        };
        let ten = RouterConfig {
            split_tax_bps: 10,
            ..RouterConfig::default()
        };
        // Up to ~80 bps of price impact on one shard (2^32 here): a 10 bps
        // tax keeps orders on one or two shards; the default 3 bps tax
        // (gerfa, 2026-10-29: price over parallelism, fee simulation) splits
        // more, never more than no tax, and never a small order.
        for e in [20u32, 26, 30, 32] {
            let a = 1u64 << e;
            let (t10, t3, t0) = (
                legs(a, ten),
                legs(a, RouterConfig::default()),
                legs(a, free),
            );
            assert!(t10 <= t3 && t3 <= t0, "2^{e}: {t10} {t3} {t0}");
            assert!(t10 <= 2, "2^{e}: {t10}");
        }
        assert_eq!(legs(1 << 20, RouterConfig::default()), 1);
        // Very large orders still split widely.
        assert!(legs(1 << 38, RouterConfig::default()) >= 4);
        // A per-leg cost larger than any gain keeps everything on one shard.
        let costly = RouterConfig {
            leg_cost_out: u64::MAX / 4,
            ..RouterConfig::default()
        };
        assert_eq!(legs(1 << 36, costly), 1);
    }

    /// Price quality of power of two against the best of all eight, on shards
    /// whose prices drifted apart by random trades (as between rebalances):
    /// the output given up per order, in bps of the best plan's output.
    #[test]
    fn power_of_two_gives_up_at_most_the_regret_bound() {
        let m = cfg();
        let mut r = 99u64;
        let (mut worst, mut total, mut n) = (0f64, 0f64, 0u32);
        for world in 0..40 {
            let mut bs = books(8, 1 << 40);
            for b in bs.iter_mut() {
                let x = 1 + next(&mut r) % (1 << 33);
                b.apply(&m, 0, next(&mut r).is_multiple_of(2), x, 0)
                    .unwrap();
            }
            for k in 0..50u64 {
                let a_to_b = next(&mut r).is_multiple_of(2);
                let amount = 1 + next(&mut r) % (1 << (16 + next(&mut r) % 20));
                let rc = RouterConfig::default();
                let p2 = plan(&bs, &m, 0, a_to_b, amount, rc, world * 1000 + k).unwrap();
                let p8 = plan(
                    &bs,
                    &m,
                    0,
                    a_to_b,
                    amount,
                    RouterConfig { choices: 0, ..rc },
                    0,
                )
                .unwrap();
                let loss =
                    (p8.amount_out as f64 - p2.amount_out as f64) / p8.amount_out as f64 * 1e4;
                worst = worst.max(loss);
                total += loss;
                n += 1;
            }
        }
        println!(
            "power of two vs best of 8: mean {:.3} bps, worst {:.3} bps over {n} orders",
            total / f64::from(n),
            worst
        );
        // max_regret_bps (1) bounds the first leg's probe quote; allow rounding.
        assert!(worst <= 1.5, "worst {worst} bps");
    }
}

#[cfg(test)]
mod round_timing {
    use super::tests::{books, cfg};
    use super::*;

    /// Not a check: prints how long one round takes at 8 and 56 shards.
    #[test]
    #[ignore]
    fn round_time() {
        let m = cfg();
        for n in [8u8, 56] {
            let mut bs = books(n, 1 << 40);
            for (i, b) in bs.iter_mut().enumerate() {
                b.apply(&m, 0, i % 2 == 0, (1 << 35) + (i as u64) * (1 << 30), 0)
                    .unwrap();
            }
            let t = std::time::Instant::now();
            let r = rebalance_round(&bs, &m, 0, [1, 1], 4);
            println!("{n} shards: {} pairs in {:?}", r.len(), t.elapsed());
        }
    }
}
