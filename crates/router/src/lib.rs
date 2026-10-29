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
//!   cheapest and the dearest shard (ADR-0006), for any keeper or bot.
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

use sharded_amm::constants::MAX_SWAP_TICK_ARRAYS;
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
}

impl Book {
    pub fn from_state(id: u8, s: &ShardState, arrays: Vec<TickArray>) -> Result<Self, AmmError> {
        Ok(Self {
            id,
            pool: Pool::load(s)?,
            fee_state: s.dynamic_fee,
            arrays,
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

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            max_shards: 8,
            min_gain_bps: 2,
            split_tax_bps: 10,
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

/// `d` distinct indices of `books`, drawn with probability proportional to
/// active liquidity (uniform if all are empty).
fn sample(books: &[Book], d: usize, seed: u64) -> Vec<usize> {
    let mut rng = seed;
    let mut left: Vec<usize> = (0..books.len()).collect();
    let mut out = Vec::with_capacity(d);
    while out.len() < d && !left.is_empty() {
        let w: Vec<u128> = left
            .iter()
            .map(|&i| books[i].pool.liquidity.max(1))
            .collect();
        let total: u128 = w.iter().sum();
        let mut r = u128::from(next(&mut rng)) % total;
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

/// Threshold routing: the smallest number of shards after which one more
/// shard would add less than `min_gain_bps + split_tax_bps` (and
/// `leg_cost_out`) to the output. The first shard is chosen by power of two
/// choices with `seed`; the others follow by quote.
pub fn plan(
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
            let best = rank[0].0;
            let regret_ok = u128::from(best) * 10_000
                <= u128::from(q) * (10_000 + u128::from(cfg.max_regret_bps));
            if regret_ok {
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
    let lo = books.iter().min_by_key(|b| (b.pool.sqrt_price, b.id))?;
    let hi = books
        .iter()
        .max_by_key(|b| (b.pool.sqrt_price, u8::MAX - b.id))?;
    if lo.pool.sqrt_price >= hi.pool.sqrt_price {
        return None;
    }
    let mut best: Option<(u64, bool, u64)> = None;
    for input_is_a in [false, true] {
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
            continue;
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
        let (x, p) = (a..=b)
            .chain([1u64 << be])
            .map(|x| (x, f(x)))
            .max_by_key(|e| e.1)
            .unwrap();
        if best.is_none_or(|(_, _, q)| p > q) {
            best = Some((x, input_is_a, p));
        }
    }
    let (amount_in, input_is_a, profit) = best?;
    (profit >= min_profit.max(1)).then(|| RebalancePlan {
        lo: lo.id,
        hi: hi.id,
        input_is_a,
        amount_in,
        profit,
        lo_array_starts: lo.array_starts(m.tick_spacing, false),
        hi_array_starts: hi.array_starts(m.tick_spacing, true),
    })
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
                };
                let mut refs: Vec<&mut TickArray> = arrays.iter_mut().collect();
                modify_position(&mut pool, &mut refs, SPACING, &mut pos, liquidity as i128)
                    .unwrap();
                Book {
                    id,
                    pool,
                    fee_state: DynamicFeeState::default(),
                    arrays,
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
        // Up to ~80 bps of price impact on one shard (2^32 here).
        for e in [20u32, 26, 30, 32] {
            let a = 1u64 << e;
            assert!(legs(a, RouterConfig::default()) <= legs(a, free), "2^{e}");
            assert!(
                legs(a, RouterConfig::default()) <= 2,
                "2^{e}: {}",
                legs(a, RouterConfig::default())
            );
        }
        // Very large orders still split widely.
        assert!(legs(1 << 38, RouterConfig::default()) >= 4);
        // A per-leg cost larger than any gain keeps everything on one shard.
        let costly = RouterConfig {
            leg_cost_out: u64::MAX / 4,
            ..RouterConfig::default()
        };
        assert_eq!(legs(1 << 36, costly), 1);
    }
}
