//! Model-based random sequences for the shard engine.
//!
//! A ledger tracks what each vault must hold; a plain list of positions is
//! the reference for active liquidity and per-tick sums. After every
//! operation the invariants below must hold, a failed operation must leave
//! every byte unchanged, and at the end every LP and the protocol must be
//! able to withdraw everything they are owed from the vaults, in any order.
//!
//! Solvency (audit F-05) is checked exactly after every operation: each vault
//! must cover the principal every position would get by removing all its
//! liquidity now, the fees already credited (`owed`), the fees earned since
//! the position's last checkpoint (computed read-only from the ticks, the
//! same way the next poke would credit them) and the protocol fees. On a
//! random share of operations, a copy of the world also exits completely in
//! a random order to prove those claims can really be paid.
//!
//! Half of the worlds run the per-shard dynamic fee (ADR-0005), so every
//! invariant is also checked with a fee that changes from swap to swap.
//!
//! Scale with AMM_STRESS_CASES / AMM_STRESS_SEED.

use anchor_lang::prelude::Pubkey;
use sharded_amm::constants::MAX_DYNAMIC_FEE_RATE_PPM;
use sharded_amm::engine::{dynamic_fee, modify_position, swap, Pool, PositionState, SwapParams};
use sharded_amm::errors::AmmError;
use sharded_amm::math::fee_math::{fees_since, growth_inside};
use sharded_amm::math::liquidity_math::amounts_for_liquidity;
use sharded_amm::math::tick_math::sqrt_price_at_tick;
use sharded_amm::math::wide::U256;
use sharded_amm::state::{
    array_span, array_start_for, DynamicFeeParams, DynamicFeeState, TickArray,
};

const MAX_SWAP_ARRAYS: usize = 3;

thread_local! {
    /// (successful swaps, ticks crossed, max crossed in one swap, failed swaps)
    static STATS: std::cell::RefCell<(u64, u64, u64, u64)> = const { std::cell::RefCell::new((0, 0, 0, 0)) };
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn log_u64(&mut self, max_bits: u32) -> u64 {
        let bits = 1 + self.below(u64::from(max_bits)) as u32;
        self.next() >> (64 - bits)
    }
}

fn cases() -> u64 {
    std::env::var("AMM_STRESS_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

fn seed() -> u64 {
    std::env::var("AMM_STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x0005_eed0_fa11)
}

#[derive(Clone)]
struct World {
    spacing: u16,
    fee: u32,
    share: u16,
    /// Dynamic fee parameters, state and the current slot.
    dynamic: DynamicFeeParams,
    fee_state: DynamicFeeState,
    slot: u64,
    pool: Pool,
    arrays: Vec<TickArray>, // contiguous, ascending start
    positions: Vec<PositionState>,
    vault_a: u128,
    vault_b: u128,
}

impl World {
    fn new(r: &mut Rng, p01: bool) -> Self {
        let spacing = [1u16, 10, 60, 200][r.below(4) as usize];
        let span = array_span(spacing);
        let centre_tick =
            (r.below(400_001) as i32 - 200_000) / i32::from(spacing) * i32::from(spacing);
        let first = array_start_for(centre_tick, spacing) - 3 * span;
        let arrays = (0..7)
            .map(|i| TickArray::new(Pubkey::default(), first + i * span, 0))
            .collect();
        let s0 = sqrt_price_at_tick(centre_tick).unwrap();
        let s1 = sqrt_price_at_tick(centre_tick + 1).unwrap();
        // P-01 also runs the 10% maximum fee.
        let fee =
            [0, 100, 500, 1000, 3000, 10_000, 100_000][r.below(if p01 { 7 } else { 6 }) as usize];
        let share = [0, 2000, 5000][r.below(3) as usize];
        let dynamic = if fee < MAX_DYNAMIC_FEE_RATE_PPM && r.below(2) == 0 {
            let filter = 1 + r.below(20) as u16;
            DynamicFeeParams {
                max_fee_rate_ppm: fee
                    + 1
                    + r.below(u64::from(MAX_DYNAMIC_FEE_RATE_PPM - fee)) as u32,
                vol_coeff_ppm: r.below(2_000) as u32,
                load_coeff_ppm: r.below(500) as u32,
                max_vol_accumulator: r.below(1_000_000) as u32,
                filter_slots: filter,
                decay_slots: filter + r.below(200) as u16,
                reduction_bps: r.below(10_001) as u16,
            }
        } else {
            DynamicFeeParams::disabled(fee)
        };
        assert_eq!(dynamic.validate(fee), Ok(()));
        Self {
            spacing,
            fee,
            share,
            dynamic,
            fee_state: DynamicFeeState::default(),
            slot: r.below(1_000_000),
            pool: Pool::at_price(s0 + u128::from(r.next()) % (s1 - s0)).unwrap(),
            arrays,
            positions: Vec::new(),
            vault_a: 0,
            vault_b: 0,
        }
    }

    fn lo(&self) -> i32 {
        self.arrays[0].start_tick_index
    }
    fn hi(&self) -> i32 {
        self.arrays.last().unwrap().start_tick_index + array_span(self.spacing)
    }

    fn random_range(&self, r: &mut Rng) -> Option<(i32, i32)> {
        let sp = i32::from(self.spacing);
        let slots = (self.hi() - self.lo()) / sp;
        // Bias toward the current price so swaps cross real ticks.
        let near = (self.pool.tick - self.lo()) / sp;
        let pick = |r: &mut Rng| -> i32 {
            if r.below(2) == 0 {
                (near + r.below(41) as i32 - 20).clamp(0, slots - 1)
            } else {
                r.below(slots as u64) as i32
            }
        };
        let (a, b) = (pick(r), pick(r));
        // Upper stays below the last slot so it is inside an existing array.
        let (a, b) = (a.min(b), (a.max(b) + 1).min(slots - 1));
        (a < b).then(|| (self.lo() + a * sp, self.lo() + b * sp))
    }

    fn modify(&mut self, i: usize, delta: i128) -> Result<(u64, u64), AmmError> {
        let mut refs: Vec<&mut TickArray> = self.arrays.iter_mut().collect();
        let mut pos = self.positions[i];
        let r = modify_position(&mut self.pool, &mut refs, self.spacing, &mut pos, delta)?;
        self.positions[i] = pos;
        Ok(r)
    }

    fn swap(&mut self, a_to_b: bool, amount: u64, limit: u128) -> Result<(u64, u64), AmmError> {
        let start = array_start_for(self.pool.tick, self.spacing);
        let idx = self
            .arrays
            .iter()
            .position(|a| a.start_tick_index == start)
            .ok_or(AmmError::TickArrayMismatch)?;
        let mut refs: Vec<&mut TickArray> = if a_to_b {
            self.arrays[..=idx]
                .iter_mut()
                .rev()
                .take(MAX_SWAP_ARRAYS)
                .collect()
        } else {
            self.arrays[idx..]
                .iter_mut()
                .take(MAX_SWAP_ARRAYS)
                .collect()
        };
        // A few swaps share a slot, the rest are spread out.
        let slot = self.slot + amount % 4;
        let mut fee_state = self.fee_state;
        let fee_rate_ppm = dynamic_fee::begin(
            &mut fee_state,
            &self.dynamic,
            self.fee,
            self.spacing,
            self.pool.tick,
            slot,
        );
        assert!(fee_rate_ppm >= self.fee);
        assert!(fee_rate_ppm <= self.fee.max(self.dynamic.max_fee_rate_ppm));
        let p = SwapParams {
            a_to_b,
            amount_in: amount,
            sqrt_price_limit: limit,
            fee_rate_ppm,
            protocol_share_bps: self.share,
            tick_spacing: self.spacing,
        };
        let o = swap(&mut self.pool, &mut refs, p)?;
        // Like the program, the fee state changes only if the swap succeeds.
        dynamic_fee::finish(
            &mut fee_state,
            &self.dynamic,
            self.spacing,
            self.pool.tick,
            slot,
        );
        self.fee_state = fee_state;
        self.slot = slot;
        STATS.with(|c| {
            let mut c = c.borrow_mut();
            c.0 += 1;
            c.1 += u64::from(o.ticks_crossed);
            c.2 = c.2.max(u64::from(o.ticks_crossed));
        });
        assert_eq!(o.amount_in, amount);
        assert!(o.protocol_fee <= o.total_fee);
        Ok((o.amount_in, o.amount_out))
    }

    /// Fees a position has earned since its checkpoint and not yet credited,
    /// computed without touching any state.
    fn pending(&self, q: &PositionState) -> (u64, u64) {
        if q.liquidity == 0 {
            return (0, 0);
        }
        let tick = |t: i32| {
            *self
                .arrays
                .iter()
                .find(|a| a.contains(t, self.spacing))
                .unwrap()
                .tick(t, self.spacing)
                .unwrap()
        };
        let (lo, hi) = (tick(q.lower), tick(q.upper));
        let p = &self.pool;
        let ia = growth_inside(
            q.lower,
            q.upper,
            p.tick,
            p.growth_a,
            lo.outside_a(),
            hi.outside_a(),
        )
        .unwrap();
        let ib = growth_inside(
            q.lower,
            q.upper,
            p.tick,
            p.growth_b,
            lo.outside_b(),
            hi.outside_b(),
        )
        .unwrap();
        (
            fees_since(q.liquidity, ia, q.inside_last_a, q.credited_a).unwrap(),
            fees_since(q.liquidity, ib, q.inside_last_b, q.credited_b).unwrap(),
        )
    }

    /// Removes everything and pays every claim in `order`; `fees_first`
    /// collects all fees before any liquidity is removed, `protocol_first`
    /// pays the protocol before the LPs. Panics if a vault would go negative.
    /// Returns the dust left in the vaults.
    fn exit_all(mut self, order: &[usize], fees_first: bool, protocol_first: bool) -> (u128, u128) {
        let pay = |w: &mut World, a: u64, b: u64, what: &str| {
            w.vault_a = w
                .vault_a
                .checked_sub(u128::from(a))
                .unwrap_or_else(|| panic!("{what} A"));
            w.vault_b = w
                .vault_b
                .checked_sub(u128::from(b))
                .unwrap_or_else(|| panic!("{what} B"));
        };
        if protocol_first {
            let (a, b) = (self.pool.protocol_a, self.pool.protocol_b);
            pay(&mut self, a, b, "protocol");
            (self.pool.protocol_a, self.pool.protocol_b) = (0, 0);
        }
        let collect = |w: &mut World, i: usize| {
            w.modify(i, 0).unwrap();
            let (a, b) = (w.positions[i].owed_a, w.positions[i].owed_b);
            pay(w, a, b, "fees");
            (w.positions[i].owed_a, w.positions[i].owed_b) = (0, 0);
        };
        if fees_first {
            for &i in order {
                collect(&mut self, i);
            }
        }
        for &i in order {
            let l = self.positions[i].liquidity as i128;
            let (a, b) = self.modify(i, -l).unwrap();
            pay(&mut self, a, b, "principal");
            collect(&mut self, i);
        }
        if !protocol_first {
            let (a, b) = (self.pool.protocol_a, self.pool.protocol_b);
            pay(&mut self, a, b, "protocol");
        }
        assert_eq!(self.pool.liquidity, 0);
        assert!(self.arrays.iter().all(|a| a.initialized_count == 0));
        (self.vault_a, self.vault_b)
    }

    fn snapshot(&self) -> (Pool, Vec<TickArray>, Vec<PositionState>) {
        (self.pool, self.arrays.clone(), self.positions.clone())
    }

    /// Checks every invariant and returns the dust `vault - claims` per token.
    fn check(&self, ctx: &str) -> (u128, u128) {
        let p = &self.pool;
        // Price and tick agree.
        let (s_t, s_t1) = (
            sqrt_price_at_tick(p.tick).unwrap(),
            sqrt_price_at_tick(p.tick + 1).unwrap(),
        );
        assert!(
            s_t <= p.sqrt_price && p.sqrt_price <= s_t1,
            "{ctx}: tick {} price {}",
            p.tick,
            p.sqrt_price
        );
        // Active liquidity equals the reference sum.
        let active: u128 = self
            .positions
            .iter()
            .filter(|q| q.lower <= p.tick && p.tick < q.upper)
            .map(|q| q.liquidity)
            .sum();
        assert_eq!(p.liquidity, active, "{ctx}: active liquidity");
        // Every tick's gross/net equals the reference; outside <= global.
        for a in &self.arrays {
            let mut count = 0;
            for (i, t) in a.ticks.iter().enumerate() {
                let tick = a.start_tick_index + i as i32 * i32::from(self.spacing);
                let gross: u128 = self
                    .positions
                    .iter()
                    .filter(|q| q.lower == tick || q.upper == tick)
                    .map(|q| q.liquidity)
                    .sum();
                let net: i128 = self
                    .positions
                    .iter()
                    .map(|q| {
                        let l = q.liquidity as i128;
                        (if q.lower == tick { l } else { 0 })
                            - (if q.upper == tick { l } else { 0 })
                    })
                    .sum();
                assert_eq!(t.gross(), gross, "{ctx}: gross at {tick}");
                assert_eq!(t.net(), net, "{ctx}: net at {tick}");
                if gross > 0 {
                    count += 1;
                    assert!(
                        t.outside_a() <= p.growth_a && t.outside_b() <= p.growth_b,
                        "{ctx}: outside > global"
                    );
                } else {
                    assert_eq!(*t, Default::default(), "{ctx}: empty tick not cleared");
                }
            }
            assert_eq!(a.initialized_count, count, "{ctx}: initialized_count");
        }
        // Vaults cover principal (as paid on removal), credited fees, fees
        // earned since the last checkpoint and protocol fees.
        let (mut need_a, mut need_b) = (u128::from(p.protocol_a), u128::from(p.protocol_b));
        for q in &self.positions {
            let (sa, sb) = (
                sqrt_price_at_tick(q.lower).unwrap(),
                sqrt_price_at_tick(q.upper).unwrap(),
            );
            let (a, b) = amounts_for_liquidity(p.sqrt_price, sa, sb, q.liquidity, false).unwrap();
            let (fa, fb) = self.pending(q);
            need_a += u128::from(a) + u128::from(q.owed_a) + u128::from(fa);
            need_b += u128::from(b) + u128::from(q.owed_b) + u128::from(fb);
        }
        assert!(
            self.vault_a >= need_a,
            "{ctx}: vault A {} < {need_a}",
            self.vault_a
        );
        assert!(
            self.vault_b >= need_b,
            "{ctx}: vault B {} < {need_b}",
            self.vault_b
        );
        (self.vault_a - need_a, self.vault_b - need_b)
    }
}

/// One random world. `p01` (exit liveness, mainnet analysis P-01): L
/// log-uniform up to 2^100, fees up to 10%, 10 000 operations, and after
/// every one of them a copy of the world must let everyone exit.
fn run_case(case: u64, r: &mut Rng, p01: bool) {
    let mut w = World::new(r, p01);
    if p01 {
        // A wide base position, as large as the u64 vaults allow, so the
        // active liquidity is rarely zero during the long run.
        let sp = i32::from(w.spacing);
        w.positions.push(PositionState {
            lower: w.lo(),
            upper: w.hi() - sp,
            ..Default::default()
        });
        let mut l: u128 = 1 << (60 + r.below(41));
        loop {
            let mut t = w.clone();
            match t.modify(0, l as i128) {
                Ok((a, b))
                    if u128::from(a.max(b)) <= u128::from(u64::MAX) / 4 && (a > 0 || b > 0) =>
                {
                    w = t;
                    w.vault_a += u128::from(a);
                    w.vault_b += u128::from(b);
                    break;
                }
                _ if l > 1 => l /= 2,
                other => panic!("base position: {other:?}"),
            }
        }
    }
    let owners = 1 + r.below(6) as usize;
    let ops = if p01 { 10_000 } else { 20 + r.below(80) };
    for op in 0..ops {
        let ctx = format!("case {case} op {op}");
        let before = w.snapshot();
        let saved = w.clone();
        let kind = r.below(10);
        if std::env::var("AMM_TRACE").is_ok() {
            eprintln!(
                "{ctx}: kind {kind} tick {} price {} L {}",
                w.pool.tick, w.pool.sqrt_price, w.pool.liquidity
            );
        }
        match kind {
            0..=2 => {
                // Open or add to a position.
                let Some((lower, upper)) = w.random_range(r) else {
                    continue;
                };
                let i = match w
                    .positions
                    .iter()
                    .position(|q| q.lower == lower && q.upper == upper)
                {
                    Some(i) if r.below(2) == 0 => i,
                    _ if w.positions.len() < owners * 3 => {
                        w.positions.push(PositionState {
                            lower,
                            upper,
                            ..Default::default()
                        });
                        w.positions.len() - 1
                    }
                    _ => continue,
                };
                let mut l = if p01 {
                    let bits = 1 + r.below(100) as u32;
                    ((u128::from(r.next()) << 64 | u128::from(r.next())) >> (128 - bits)).max(1)
                } else {
                    u128::from(r.log_u64(60))
                };
                // P-01: halve a huge L until the deposit fits the u64 vaults,
                // so large liquidity actually lands instead of reverting.
                while p01 && l > 1 {
                    let mut t = w.clone();
                    match t.modify(i, l as i128) {
                        Ok((a, b))
                            if w.vault_a + u128::from(a) <= u128::from(u64::MAX) / 2
                                && w.vault_b + u128::from(b) <= u128::from(u64::MAX) / 2 =>
                        {
                            break
                        }
                        _ => l /= 2,
                    }
                }
                match w.modify(i, l as i128) {
                    Ok((a, b)) => {
                        w.vault_a += u128::from(a);
                        w.vault_b += u128::from(b);
                        if l > 0 {
                            assert!(a > 0 || b > 0, "{ctx}: free liquidity");
                        }
                    }
                    Err(e) => {
                        assert!(
                            matches!(
                                e,
                                AmmError::MathOverflow | AmmError::LiquidityPerTickExceeded
                            ),
                            "{ctx}: {e:?}"
                        );
                        if w.positions[i].liquidity == 0 && before.2.len() < w.positions.len() {
                            w.positions.pop();
                        }
                        assert!(
                            w.pool == before.0 && w.arrays == before.1,
                            "{ctx}: failed add mutated state"
                        );
                    }
                }
            }
            3 | 4 if !w.positions.is_empty() => {
                // Remove part or all of a position; never fails while L suffices.
                let i = r.below(w.positions.len() as u64) as usize;
                let have = w.positions[i].liquidity;
                // P-01 keeps its base position (the exit checks still
                // withdraw it, in a copy, after every operation).
                if have == 0 || (p01 && i == 0) {
                    continue;
                }
                let take = if r.below(3) == 0 {
                    have
                } else {
                    1 + u128::from(r.next()) % have
                };
                let (a, b) = w
                    .modify(i, -(take as i128))
                    .unwrap_or_else(|e| panic!("{ctx}: remove {e:?}"));
                w.vault_a = w
                    .vault_a
                    .checked_sub(u128::from(a))
                    .expect("vault A underflow");
                w.vault_b = w
                    .vault_b
                    .checked_sub(u128::from(b))
                    .expect("vault B underflow");
                // Over-removal is rejected without changes.
                let snap = w.snapshot();
                let too_much = w.positions[i].liquidity as i128 + 1;
                assert_eq!(
                    w.modify(i, -too_much),
                    Err(AmmError::InsufficientPositionLiquidity)
                );
                assert!(w.pool == snap.0 && w.arrays == snap.1 && w.positions == snap.2);
            }
            5 if !w.positions.is_empty() => {
                // Collect fees.
                let i = r.below(w.positions.len() as u64) as usize;
                w.modify(i, 0)
                    .unwrap_or_else(|e| panic!("{ctx}: poke {e:?}"));
                let q = &mut w.positions[i];
                w.vault_a = w
                    .vault_a
                    .checked_sub(u128::from(q.owed_a))
                    .expect("fee A underflow");
                w.vault_b = w
                    .vault_b
                    .checked_sub(u128::from(q.owed_b))
                    .expect("fee B underflow");
                q.owed_a = 0;
                q.owed_b = 0;
            }
            _ => {
                let a_to_b = r.below(2) == 0;
                let amount = r.log_u64(56);
                let limit = if r.below(4) == 0 {
                    let t = w.pool.tick
                        + if a_to_b {
                            -(r.below(500) as i32)
                        } else {
                            1 + r.below(500) as i32
                        };
                    sqrt_price_at_tick(t.clamp(-443_635, 443_635)).unwrap()
                } else {
                    0
                };
                // P-01: stay inside the base position, so the long run keeps
                // trading instead of parking the price at a liquidity gap.
                let limit = if p01 && limit == 0 {
                    let sp = i32::from(w.spacing);
                    let edge = if a_to_b { w.lo() + sp } else { w.hi() - 2 * sp };
                    sqrt_price_at_tick(edge).unwrap()
                } else {
                    limit
                };
                let price_before = w.pool.sqrt_price;
                let res = w.swap(a_to_b, amount, limit);
                if std::env::var("AMM_TRACE").is_ok() {
                    eprintln!("  swap a_to_b={a_to_b} amount={amount} limit={limit} -> {res:?} tick {} price {} L {}", w.pool.tick, w.pool.sqrt_price, w.pool.liquidity);
                    let ticks: Vec<(i32, i128)> = w
                        .arrays
                        .iter()
                        .flat_map(|a| {
                            a.ticks
                                .iter()
                                .enumerate()
                                .filter(|(_, t)| t.gross() > 0)
                                .map(move |(i, t)| {
                                    (
                                        a.start_tick_index + i as i32 * i32::from(w.spacing),
                                        t.net(),
                                    )
                                })
                        })
                        .collect();
                    eprintln!("  spacing {} ticks {ticks:?}", w.spacing);
                }
                match res {
                    Ok((inp, out)) => {
                        if a_to_b {
                            w.vault_a += u128::from(inp);
                            w.vault_b = w
                                .vault_b
                                .checked_sub(u128::from(out))
                                .expect("swap B underflow");
                            assert!(w.pool.sqrt_price <= price_before);
                        } else {
                            w.vault_b += u128::from(inp);
                            w.vault_a = w
                                .vault_a
                                .checked_sub(u128::from(out))
                                .expect("swap A underflow");
                            assert!(w.pool.sqrt_price >= price_before);
                        }
                        // Immediately swapping the output back never returns more.
                        let mut probe = w.clone();
                        if let Ok((_, back)) = probe.swap(!a_to_b, out, 0) {
                            assert!(back <= inp, "{ctx}: round trip {inp} -> {out} -> {back}");
                        }
                    }
                    Err(e) => {
                        STATS.with(|c| c.borrow_mut().3 += 1);
                        assert!(
                            matches!(
                                e,
                                AmmError::ZeroLiquidityGap
                                    | AmmError::TickArraysExhausted
                                    | AmmError::PriceLimitReached
                                    | AmmError::ZeroOutput
                                    | AmmError::InvalidPriceLimit
                                    | AmmError::MathOverflow
                                    | AmmError::TickArrayMismatch
                                    | AmmError::TooManySwapSteps
                                    | AmmError::ZeroAmount
                            ),
                            "{ctx}: unexpected {e:?}"
                        );
                        assert!(
                            w.pool == before.0 && w.arrays == before.1,
                            "{ctx}: failed swap mutated state"
                        );
                    }
                }
            }
        }
        // SPL balances are u64: an operation that would push a vault past
        // u64::MAX fails in the token program, so the model reverts it.
        if w.vault_a > u128::from(u64::MAX) || w.vault_b > u128::from(u64::MAX) {
            w = saved;
            continue;
        }
        let dust = w.check(&ctx);
        if p01 || r.below(8) == 0 {
            let mut order: Vec<usize> = (0..w.positions.len()).collect();
            for i in (1..order.len()).rev() {
                order.swap(i, r.below(i as u64 + 1) as usize);
            }
            // Paying every claim leaves exactly the dust: the claims the
            // check counts are exactly what an exit pays, in any order.
            let left = w.clone().exit_all(&order, r.below(2) == 0, r.below(2) == 0);
            assert_eq!(left, dust, "{ctx}: exit paid a different amount");
        }
    }
    // Everyone exits, in several orders: forward, reverse and random, with
    // fees collected first or last and the protocol paid first or last.
    // Every order must succeed and leave exactly the dust the check counts.
    let dust = w.check("final");
    let n = w.positions.len();
    let forward: Vec<usize> = (0..n).collect();
    let mut reverse = forward.clone();
    reverse.reverse();
    let mut shuffled = forward.clone();
    for i in (1..n).rev() {
        shuffled.swap(i, r.below(i as u64 + 1) as usize);
    }
    for (order, fees_first, protocol_first) in [
        (&forward, false, false),
        (&reverse, true, false),
        (&shuffled, false, true),
        (&shuffled, true, true),
    ] {
        assert_eq!(w.clone().exit_all(order, fees_first, protocol_first), dust);
    }
    let _ = U256::zero();
}

#[test]
fn random_sequences_keep_every_invariant() {
    let mut r = Rng(seed() | 1);
    for case in 0..cases() {
        run_case(case, &mut r, false);
    }
    let (ok, crossed, max, failed) = STATS.with(|c| *c.borrow());
    eprintln!("swaps ok {ok}, failed {failed}, ticks crossed {crossed}, max in one swap {max}");
    assert!(
        ok > failed / 4 && crossed > ok / 10,
        "workload too easy: {ok} ok, {failed} failed, {crossed} crossed"
    );
}

/// P-01: exit liveness on long, large worlds. Every position can always
/// withdraw everything and collect, in any order, after every operation.
#[test]
fn exit_always_works_on_long_large_worlds() {
    let mut r = Rng(seed().wrapping_mul(0x9e37_79b9) | 1);
    let worlds = (cases() / 20_000).max(2);
    for case in 0..worlds {
        run_case(case, &mut r, true);
    }
    let (ok, crossed, _, failed) = STATS.with(|c| *c.borrow());
    eprintln!("STATS p01 worlds {worlds} x 10000 operations: swaps ok {ok}, failed {failed}, ticks crossed {crossed}");
    assert!(ok > 1_000 * worlds && crossed > 0, "P-01 workload too easy");
}
