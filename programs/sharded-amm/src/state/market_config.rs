//! `MarketConfig`: one token pair, its parameters and roles.
//!
//! Read-only during swaps and LP operations. Mint order is canonical
//! (`mint_a < mint_b` by bytes), so a pair has exactly one market per
//! `market_id`. `tick_spacing` is immutable once the market exists: changing
//! it would invalidate every position's tick alignment.

use anchor_lang::prelude::*;

use crate::constants::{
    FEE_TIMELOCK_SECONDS, MAX_ACTIVATION_DEVIATION_BPS, MAX_DEPTH_IMPACT_BPS,
    MAX_DYNAMIC_COEFF_PPM, MAX_DYNAMIC_FEE_RATE_PPM, MAX_FEE_RATE_PPM, MAX_PROTOCOL_SHARE_BPS,
    MAX_REBALANCERS, MAX_TICK_SPACING, MAX_VOL_ACCUMULATOR, PAUSE_DEPOSITS, PAUSE_FLAGS_MASK,
    PAUSE_REBALANCE, PAUSE_SWAPS,
};
use crate::errors::AmmError;
use crate::state::MarketStatus;

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct MarketConfig {
    pub schema_version: u16,
    pub market_id: u64,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    /// Pinned token program (classic SPL Token in v1).
    pub token_program: Pubkey,
    /// Parameters within bounds. Cannot move principal.
    pub admin: Pubkey,
    /// Prepares/activates shards only when on-chain readiness checks pass.
    pub keeper: Pubkey,
    /// Collects accrued protocol fees only.
    pub revenue_authority: Pubkey,
    /// Pauses risky operations; never blocks valid LP exits.
    pub pauser: Pubkey,
    pub params: MarketParams,
    /// Dynamic fee bounds and coefficients (ADR-0005); state is per shard.
    pub dynamic_fees: DynamicFeeParams,
    pub status: MarketStatus,
    pub bump: u8,
    /// Thresholds a shard must meet, re-checked on-chain, before it is routable.
    pub readiness: ReadinessParams,
    /// Fee change waiting for `FEE_TIMELOCK_SECONDS`.
    pub pending_fees: PendingFees,
    /// Selective pauses (`PAUSE_*`, ADR-0006); none blocks LP exits.
    pub pause_flags: u8,
    /// The only keys that may call `rebalance_pair` (ADR-0006). Starts as
    /// `[keeper, empty, empty, empty]`.
    pub rebalancers: [Pubkey; MAX_REBALANCERS],
    /// Rebalancer change waiting for `FEE_TIMELOCK_SECONDS`.
    pub pending_rebalancers: PendingRebalancers,
    /// Zeroed space for future fields without a realloc.
    pub reserved: [u8; 14],
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct ReadinessParams {
    /// Minimum active liquidity at the shard's current price.
    pub min_liquidity: u128,
    /// Minimum raw vault balances, both sides.
    pub min_reserve_a: u64,
    pub min_reserve_b: u64,
    /// Expansion only: max price difference from an active reference shard.
    pub max_activation_deviation_bps: u16,
    /// Usable depth both ways (`verify_depth`): selling `depth_probe_a` of A
    /// and, separately, `depth_probe_b` of B must each fill completely within
    /// `max_depth_impact_bps` ticks (1 tick ~ 1 bp of price).
    pub depth_probe_a: u64,
    pub depth_probe_b: u64,
    pub max_depth_impact_bps: u16,
}

impl ReadinessParams {
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        if self.min_liquidity == 0
            || self.min_reserve_a == 0
            || self.min_reserve_b == 0
            || self.max_activation_deviation_bps > MAX_ACTIVATION_DEVIATION_BPS
            || self.depth_probe_a == 0
            || self.depth_probe_b == 0
            || self.max_depth_impact_bps == 0
            || self.max_depth_impact_bps > MAX_DEPTH_IMPACT_BPS
        {
            return Err(AmmError::InvalidReadinessParams);
        }
        Ok(())
    }

    /// Liquidity and both real vault balances meet the thresholds.
    pub fn is_ready(&self, liquidity: u128, vault_a: u64, vault_b: u64) -> bool {
        liquidity >= self.min_liquidity
            && vault_a >= self.min_reserve_a
            && vault_b >= self.min_reserve_b
    }
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct PendingFees {
    pub fee_rate_ppm: u32,
    pub protocol_share_bps: u16,
    pub dynamic: DynamicFeeParams,
    /// Earliest unix time at which the change can be applied.
    pub eta: i64,
    pub active: bool,
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct PendingRebalancers {
    pub keys: [Pubkey; MAX_REBALANCERS],
    /// Earliest unix time at which the change can be applied.
    pub eta: i64,
    pub active: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarketParams {
    /// Swap fee in parts per million of input.
    pub fee_rate_ppm: u32,
    /// Protocol share of the swap fee, basis points; the rest goes to LPs.
    pub protocol_share_bps: u16,
    pub tick_spacing: u16,
}

impl MarketParams {
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        if self.fee_rate_ppm > MAX_FEE_RATE_PPM {
            return Err(AmmError::InvalidFeeRate);
        }
        if self.protocol_share_bps > MAX_PROTOCOL_SHARE_BPS {
            return Err(AmmError::InvalidProtocolShare);
        }
        if self.tick_spacing == 0 || self.tick_spacing > MAX_TICK_SPACING {
            return Err(AmmError::InvalidTickSpacing);
        }
        Ok(())
    }
}

/// Dynamic fee parameters (ADR-0005). `fee_rate_ppm` of `MarketParams` is
/// the base and the minimum; `max_fee_rate_ppm == fee_rate_ppm` turns the
/// dynamic part off.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct DynamicFeeParams {
    pub max_fee_rate_ppm: u32,
    /// Fee added per `tick_spacing` of accumulated volatility.
    pub vol_coeff_ppm: u32,
    /// Fee added per swap already executed in the same slot.
    pub load_coeff_ppm: u32,
    pub max_vol_accumulator: u32,
    /// Slots after the last swap during which the episode continues.
    pub filter_slots: u16,
    /// Slots after the last swap after which nothing carries over.
    pub decay_slots: u16,
    /// Share of the accumulator carried into the next episode.
    pub reduction_bps: u16,
}

impl DynamicFeeParams {
    /// Dynamic part off: the fee is always `base`.
    pub const fn disabled(base: u32) -> Self {
        Self {
            max_fee_rate_ppm: base,
            vol_coeff_ppm: 0,
            load_coeff_ppm: 0,
            max_vol_accumulator: 0,
            filter_slots: 1,
            decay_slots: 1,
            reduction_bps: 0,
        }
    }

    pub fn is_enabled(&self, base: u32) -> bool {
        self.max_fee_rate_ppm > base
    }

    pub fn validate(&self, base: u32) -> std::result::Result<(), AmmError> {
        let ceiling_ok = self.max_fee_rate_ppm == base
            || (self.max_fee_rate_ppm > base && self.max_fee_rate_ppm <= MAX_DYNAMIC_FEE_RATE_PPM);
        if !ceiling_ok
            || self.vol_coeff_ppm > MAX_DYNAMIC_COEFF_PPM
            || self.load_coeff_ppm > MAX_DYNAMIC_COEFF_PPM
            || self.max_vol_accumulator > MAX_VOL_ACCUMULATOR
            || self.filter_slots == 0
            || self.filter_slots > self.decay_slots
            || self.reduction_bps > 10_000
        {
            return Err(AmmError::InvalidDynamicFeeParams);
        }
        Ok(())
    }
}

/// Canonical order: distinct mints with `mint_a < mint_b` by bytes.
pub fn check_mint_order(mint_a: &Pubkey, mint_b: &Pubkey) -> std::result::Result<(), AmmError> {
    if mint_a.to_bytes() >= mint_b.to_bytes() {
        return Err(AmmError::MintsNotCanonical);
    }
    Ok(())
}

impl MarketConfig {
    /// `Open -> Paused` by the pauser or admin; `Paused -> Open` by the admin
    /// only, so a compromised pauser can halt but not resume. Pausing while
    /// `Initializing` is not a v1 transition.
    pub fn set_paused(
        &mut self,
        signer: &Pubkey,
        paused: bool,
    ) -> std::result::Result<(), AmmError> {
        match (self.status, paused) {
            (MarketStatus::Open, true) => {
                if *signer != self.pauser && *signer != self.admin {
                    return Err(AmmError::Unauthorized);
                }
                self.status = MarketStatus::Paused;
            }
            (MarketStatus::Paused, false) => {
                if *signer != self.admin {
                    return Err(AmmError::Unauthorized);
                }
                self.status = MarketStatus::Open;
            }
            _ => return Err(AmmError::InvalidMarketStatus),
        }
        Ok(())
    }

    /// Swaps allowed: status permits them and `PAUSE_SWAPS` is clear.
    pub fn check_swaps(&self) -> std::result::Result<(), AmmError> {
        if self.pause_flags & PAUSE_SWAPS != 0 {
            return Err(AmmError::SwapsPaused);
        }
        Ok(())
    }

    /// New liquidity allowed by status and `PAUSE_DEPOSITS`.
    pub fn check_deposits(&self) -> std::result::Result<(), AmmError> {
        if !self.status.allows_add_liquidity() {
            return Err(AmmError::InvalidMarketStatus);
        }
        if self.pause_flags & PAUSE_DEPOSITS != 0 {
            return Err(AmmError::DepositsPaused);
        }
        Ok(())
    }

    /// Rebalancing is two swaps, so both bits must be clear.
    pub fn check_rebalance(&self) -> std::result::Result<(), AmmError> {
        self.check_swaps()?;
        if self.pause_flags & PAUSE_REBALANCE != 0 {
            return Err(AmmError::RebalancePaused);
        }
        Ok(())
    }

    /// The pauser may only add bits; the admin may set any combination.
    /// Unknown bits are rejected. Returns the previous flags.
    pub fn set_pause_flags(
        &mut self,
        signer: &Pubkey,
        flags: u8,
    ) -> std::result::Result<u8, AmmError> {
        if flags & !PAUSE_FLAGS_MASK != 0 {
            return Err(AmmError::InvalidPauseFlags);
        }
        let old = self.pause_flags;
        let allowed = *signer == self.admin || (*signer == self.pauser && flags & old == old);
        if !allowed {
            return Err(AmmError::Unauthorized);
        }
        self.pause_flags = flags;
        Ok(old)
    }

    /// Admin proposes a fee change within bounds; it can be applied after
    /// `FEE_TIMELOCK_SECONDS`. `tick_spacing` is immutable and not an input.
    /// A new proposal replaces a pending one and restarts the delay.
    pub fn propose_fees(
        &mut self,
        signer: &Pubkey,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
        dynamic: DynamicFeeParams,
        now: i64,
    ) -> std::result::Result<PendingFees, AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        MarketParams {
            fee_rate_ppm,
            protocol_share_bps,
            ..self.params
        }
        .validate()?;
        dynamic.validate(fee_rate_ppm)?;
        let eta = now
            .checked_add(FEE_TIMELOCK_SECONDS)
            .ok_or(AmmError::MathOverflow)?;
        self.pending_fees = PendingFees {
            fee_rate_ppm,
            protocol_share_bps,
            dynamic,
            eta,
            active: true,
        };
        Ok(self.pending_fees)
    }

    /// Admin applies the pending change once the delay has passed.
    /// Returns `(old, new)` params for the event.
    pub fn apply_fees(
        &mut self,
        signer: &Pubkey,
        now: i64,
    ) -> std::result::Result<(MarketParams, MarketParams), AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        let p = self.pending_fees;
        if !p.active {
            return Err(AmmError::NoPendingChange);
        }
        if now < p.eta {
            return Err(AmmError::TimelockNotElapsed);
        }
        let old = self.params;
        let new = MarketParams {
            fee_rate_ppm: p.fee_rate_ppm,
            protocol_share_bps: p.protocol_share_bps,
            ..old
        };
        new.validate()?;
        p.dynamic.validate(new.fee_rate_ppm)?;
        self.params = new;
        self.dynamic_fees = p.dynamic;
        self.pending_fees = PendingFees::default();
        Ok((old, new))
    }

    pub fn cancel_fee_change(&mut self, signer: &Pubkey) -> std::result::Result<(), AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        if !self.pending_fees.active {
            return Err(AmmError::NoPendingChange);
        }
        self.pending_fees = PendingFees::default();
        Ok(())
    }

    /// Signer is a listed rebalancer; empty slots match no one.
    pub fn is_rebalancer(&self, signer: &Pubkey) -> bool {
        *signer != Pubkey::default() && self.rebalancers.contains(signer)
    }

    /// Admin proposes a new rebalancer list, applied after
    /// `FEE_TIMELOCK_SECONDS`. Empty slots are `Pubkey::default()`; an
    /// all-empty list stops rebalancing. A new proposal restarts the delay.
    pub fn propose_rebalancers(
        &mut self,
        signer: &Pubkey,
        keys: [Pubkey; MAX_REBALANCERS],
        now: i64,
    ) -> std::result::Result<PendingRebalancers, AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        for (i, k) in keys.iter().enumerate() {
            if *k != Pubkey::default() && keys[..i].contains(k) {
                return Err(AmmError::InvalidRebalancers);
            }
        }
        let eta = now
            .checked_add(FEE_TIMELOCK_SECONDS)
            .ok_or(AmmError::MathOverflow)?;
        self.pending_rebalancers = PendingRebalancers {
            keys,
            eta,
            active: true,
        };
        Ok(self.pending_rebalancers)
    }

    /// Admin applies the pending list once the delay has passed. Returns
    /// the previous list.
    pub fn apply_rebalancers(
        &mut self,
        signer: &Pubkey,
        now: i64,
    ) -> std::result::Result<[Pubkey; MAX_REBALANCERS], AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        let p = self.pending_rebalancers;
        if !p.active {
            return Err(AmmError::NoPendingChange);
        }
        if now < p.eta {
            return Err(AmmError::TimelockNotElapsed);
        }
        let old = self.rebalancers;
        self.rebalancers = p.keys;
        self.pending_rebalancers = PendingRebalancers::default();
        Ok(old)
    }

    pub fn cancel_rebalancers_change(
        &mut self,
        signer: &Pubkey,
    ) -> std::result::Result<(), AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        if !self.pending_rebalancers.active {
            return Err(AmmError::NoPendingChange);
        }
        self.pending_rebalancers = PendingRebalancers::default();
        Ok(())
    }

    /// Admin cancels a market that never opened; LPs can still withdraw.
    pub fn cancel_setup(&mut self, signer: &Pubkey) -> std::result::Result<(), AmmError> {
        if *signer != self.admin {
            return Err(AmmError::Unauthorized);
        }
        if self.status != MarketStatus::Initializing {
            return Err(AmmError::InvalidMarketStatus);
        }
        self.status = MarketStatus::Cancelled;
        Ok(())
    }

    /// Signer may prepare shards: the keeper, or the admin during setup.
    pub fn can_prepare(&self, signer: &Pubkey) -> bool {
        *signer == self.keeper
            || (self.status == MarketStatus::Initializing && *signer == self.admin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_bounds() {
        let ok = MarketParams {
            fee_rate_ppm: 3_000,
            protocol_share_bps: 2_000,
            tick_spacing: 60,
        };
        assert_eq!(ok.validate(), Ok(()));
        for fee in [500, 1_000, 3_000, 0, MAX_FEE_RATE_PPM] {
            assert_eq!(
                MarketParams {
                    fee_rate_ppm: fee,
                    ..ok
                }
                .validate(),
                Ok(())
            );
        }
        assert_eq!(
            MarketParams {
                fee_rate_ppm: MAX_FEE_RATE_PPM + 1,
                ..ok
            }
            .validate(),
            Err(AmmError::InvalidFeeRate)
        );
        assert_eq!(
            MarketParams {
                protocol_share_bps: 5_001,
                ..ok
            }
            .validate(),
            Err(AmmError::InvalidProtocolShare)
        );
        for s in [0u16, 1_001, u16::MAX] {
            assert_eq!(
                MarketParams {
                    tick_spacing: s,
                    ..ok
                }
                .validate(),
                Err(AmmError::InvalidTickSpacing)
            );
        }
        for s in [1u16, 10, 60, 1_000] {
            assert_eq!(
                MarketParams {
                    tick_spacing: s,
                    ..ok
                }
                .validate(),
                Ok(())
            );
        }
    }

    fn config(status: MarketStatus) -> MarketConfig {
        MarketConfig {
            schema_version: 1,
            market_id: 1,
            mint_a: Pubkey::new_unique(),
            mint_b: Pubkey::new_unique(),
            token_program: Pubkey::new_unique(),
            admin: Pubkey::new_from_array([1; 32]),
            keeper: Pubkey::new_from_array([2; 32]),
            revenue_authority: Pubkey::new_from_array([3; 32]),
            pauser: Pubkey::new_from_array([4; 32]),
            params: MarketParams {
                fee_rate_ppm: 3_000,
                protocol_share_bps: 2_000,
                tick_spacing: 60,
            },
            dynamic_fees: DynamicFeeParams::disabled(3_000),
            status,
            bump: 255,
            readiness: ReadinessParams {
                min_liquidity: 1,
                min_reserve_a: 1,
                min_reserve_b: 1,
                max_activation_deviation_bps: 100,
                depth_probe_a: 1,
                depth_probe_b: 1,
                max_depth_impact_bps: 100,
            },
            pending_fees: PendingFees::default(),
            pause_flags: 0,
            rebalancers: [
                Pubkey::new_from_array([2; 32]),
                Pubkey::default(),
                Pubkey::default(),
                Pubkey::default(),
            ],
            pending_rebalancers: PendingRebalancers::default(),
            reserved: [0; 14],
        }
    }

    #[test]
    fn pause_roles_and_transitions() {
        let mut c = config(MarketStatus::Open);
        let (admin, keeper, revenue, pauser) = (c.admin, c.keeper, c.revenue_authority, c.pauser);
        for who in [keeper, revenue] {
            assert_eq!(
                c.clone().set_paused(&who, true),
                Err(AmmError::Unauthorized)
            );
        }
        assert_eq!(c.set_paused(&pauser, true), Ok(()));
        assert_eq!(c.status, MarketStatus::Paused);
        assert_eq!(
            c.set_paused(&pauser, true),
            Err(AmmError::InvalidMarketStatus)
        );
        assert_eq!(c.set_paused(&pauser, false), Err(AmmError::Unauthorized));
        assert_eq!(c.set_paused(&admin, false), Ok(()));
        assert_eq!(c.status, MarketStatus::Open);
        assert_eq!(c.set_paused(&admin, true), Ok(()));
        let mut c = config(MarketStatus::Initializing);
        assert_eq!(
            c.set_paused(&pauser, true),
            Err(AmmError::InvalidMarketStatus)
        );
        assert_eq!(
            c.set_paused(&admin, false),
            Err(AmmError::InvalidMarketStatus)
        );
    }

    #[test]
    fn fee_change_is_timelocked_admin_only_and_bounded() {
        let mut c = config(MarketStatus::Open);
        let admin = c.admin;
        for who in [c.keeper, c.revenue_authority, c.pauser] {
            assert_eq!(
                c.propose_fees(&who, 500, 2_000, DynamicFeeParams::disabled(500), 0),
                Err(AmmError::Unauthorized)
            );
        }
        assert_eq!(
            c.propose_fees(
                &admin,
                100_001,
                2_000,
                DynamicFeeParams::disabled(100_001),
                0
            ),
            Err(AmmError::InvalidFeeRate)
        );
        assert_eq!(
            c.propose_fees(&admin, 500, 5_001, DynamicFeeParams::disabled(500), 0),
            Err(AmmError::InvalidProtocolShare)
        );
        assert_eq!(c.apply_fees(&admin, 0), Err(AmmError::NoPendingChange));
        c.propose_fees(&admin, 500, 1_000, DynamicFeeParams::disabled(500), 1_000)
            .unwrap();
        assert_eq!(
            c.params.fee_rate_ppm, 3_000,
            "nothing changes before the delay"
        );
        assert_eq!(
            c.apply_fees(&admin, 1_000 + FEE_TIMELOCK_SECONDS - 1),
            Err(AmmError::TimelockNotElapsed)
        );
        assert_eq!(
            c.apply_fees(&c.keeper.clone(), 1_000 + FEE_TIMELOCK_SECONDS),
            Err(AmmError::Unauthorized)
        );
        let (old, new) = c.apply_fees(&admin, 1_000 + FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(old.fee_rate_ppm, 3_000);
        assert_eq!(
            new,
            MarketParams {
                fee_rate_ppm: 500,
                protocol_share_bps: 1_000,
                tick_spacing: 60
            }
        );
        assert_eq!(c.params, new);
        assert_eq!(
            c.apply_fees(&admin, i64::MAX),
            Err(AmmError::NoPendingChange)
        );
        c.propose_fees(&admin, 0, 0, DynamicFeeParams::disabled(0), 5)
            .unwrap();
        c.cancel_fee_change(&admin).unwrap();
        assert_eq!(
            c.apply_fees(&admin, i64::MAX),
            Err(AmmError::NoPendingChange)
        );
        assert_eq!(
            c.propose_fees(&admin, 0, 0, DynamicFeeParams::disabled(0), i64::MAX),
            Err(AmmError::MathOverflow)
        );
    }

    fn dynamic(base: u32) -> DynamicFeeParams {
        DynamicFeeParams {
            max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM,
            vol_coeff_ppm: 100,
            load_coeff_ppm: 0,
            max_vol_accumulator: 100_000,
            filter_slots: 10,
            decay_slots: 120,
            reduction_bps: 5_000,
        }
        .clamp_base(base)
    }

    impl DynamicFeeParams {
        fn clamp_base(mut self, base: u32) -> Self {
            self.max_fee_rate_ppm = self.max_fee_rate_ppm.max(base);
            self
        }
    }

    #[test]
    fn pause_flags_roles_and_effects() {
        let mut c = config(MarketStatus::Open);
        let (admin, pauser, keeper) = (c.admin, c.pauser, c.keeper);
        assert_eq!(
            c.set_pause_flags(&keeper, PAUSE_SWAPS),
            Err(AmmError::Unauthorized)
        );
        assert_eq!(
            c.set_pause_flags(&admin, 1 << 3),
            Err(AmmError::InvalidPauseFlags)
        );
        assert_eq!(c.set_pause_flags(&pauser, PAUSE_REBALANCE), Ok(0));
        assert_eq!(c.check_rebalance(), Err(AmmError::RebalancePaused));
        assert_eq!(c.check_swaps(), Ok(()));
        // The pauser can add, never remove.
        assert_eq!(
            c.set_pause_flags(&pauser, PAUSE_REBALANCE | PAUSE_SWAPS),
            Ok(PAUSE_REBALANCE)
        );
        assert_eq!(c.check_swaps(), Err(AmmError::SwapsPaused));
        assert_eq!(c.check_rebalance(), Err(AmmError::SwapsPaused));
        assert_eq!(
            c.set_pause_flags(&pauser, PAUSE_SWAPS),
            Err(AmmError::Unauthorized)
        );
        assert_eq!(c.set_pause_flags(&pauser, 0), Err(AmmError::Unauthorized));
        assert_eq!(
            c.set_pause_flags(&admin, PAUSE_DEPOSITS),
            Ok(PAUSE_REBALANCE | PAUSE_SWAPS)
        );
        assert_eq!(c.check_deposits(), Err(AmmError::DepositsPaused));
        assert_eq!(c.check_swaps(), Ok(()));
        assert_eq!(c.set_pause_flags(&admin, 0), Ok(PAUSE_DEPOSITS));
        assert_eq!(c.check_deposits(), Ok(()));
    }

    #[test]
    fn dynamic_fee_params_bounds() {
        let base = 300;
        assert_eq!(dynamic(base).validate(base), Ok(()));
        assert_eq!(DynamicFeeParams::disabled(base).validate(base), Ok(()));
        assert!(!DynamicFeeParams::disabled(base).is_enabled(base));
        assert!(dynamic(base).is_enabled(base));
        // A base fee above the dynamic ceiling is allowed only with the
        // dynamic part off.
        assert_eq!(DynamicFeeParams::disabled(50_000).validate(50_000), Ok(()));
        let bad = [
            DynamicFeeParams {
                max_fee_rate_ppm: base - 1,
                ..dynamic(base)
            },
            DynamicFeeParams {
                max_fee_rate_ppm: MAX_DYNAMIC_FEE_RATE_PPM + 1,
                ..dynamic(base)
            },
            DynamicFeeParams {
                vol_coeff_ppm: MAX_DYNAMIC_COEFF_PPM + 1,
                ..dynamic(base)
            },
            DynamicFeeParams {
                load_coeff_ppm: MAX_DYNAMIC_COEFF_PPM + 1,
                ..dynamic(base)
            },
            DynamicFeeParams {
                max_vol_accumulator: MAX_VOL_ACCUMULATOR + 1,
                ..dynamic(base)
            },
            DynamicFeeParams {
                filter_slots: 0,
                ..dynamic(base)
            },
            DynamicFeeParams {
                filter_slots: 121,
                ..dynamic(base)
            },
            DynamicFeeParams {
                reduction_bps: 10_001,
                ..dynamic(base)
            },
        ];
        for p in bad {
            assert_eq!(
                p.validate(base),
                Err(AmmError::InvalidDynamicFeeParams),
                "{p:?}"
            );
        }
    }

    #[test]
    fn dynamic_fee_change_goes_through_the_timelock() {
        let mut c = config(MarketStatus::Open);
        let admin = c.admin;
        // The ceiling is checked against the proposed base, not the current one.
        assert_eq!(
            c.propose_fees(&admin, 9_000, 2_000, dynamic(300), 0),
            Err(AmmError::InvalidDynamicFeeParams)
        );
        c.propose_fees(&admin, 300, 2_000, dynamic(300), 0).unwrap();
        assert_eq!(c.dynamic_fees, DynamicFeeParams::disabled(3_000));
        c.apply_fees(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(c.dynamic_fees, dynamic(300));
        assert_eq!(c.params.fee_rate_ppm, 300);
    }

    #[test]
    fn cancel_only_before_open() {
        let mut c = config(MarketStatus::Initializing);
        assert_eq!(
            c.cancel_setup(&c.keeper.clone()),
            Err(AmmError::Unauthorized)
        );
        c.cancel_setup(&c.admin.clone()).unwrap();
        assert_eq!(c.status, MarketStatus::Cancelled);
        assert!(
            !c.status.allows_add_liquidity()
                && !c.status.allows_swap()
                && c.status.allows_lp_exit()
        );
        assert_eq!(
            c.cancel_setup(&c.admin.clone()),
            Err(AmmError::InvalidMarketStatus)
        );
        let mut open = config(MarketStatus::Open);
        assert_eq!(
            open.cancel_setup(&open.admin.clone()),
            Err(AmmError::InvalidMarketStatus)
        );
    }

    #[test]
    fn readiness_bounds() {
        let r = config(MarketStatus::Open).readiness;
        assert_eq!(r.validate(), Ok(()));
        assert!(
            r.is_ready(1, 1, 1)
                && !r.is_ready(0, 1, 1)
                && !r.is_ready(1, 0, 1)
                && !r.is_ready(1, 1, 0)
        );
        for bad in [
            ReadinessParams {
                min_liquidity: 0,
                ..r
            },
            ReadinessParams {
                min_reserve_a: 0,
                ..r
            },
            ReadinessParams {
                min_reserve_b: 0,
                ..r
            },
            ReadinessParams {
                max_activation_deviation_bps: MAX_ACTIVATION_DEVIATION_BPS + 1,
                ..r
            },
            ReadinessParams {
                depth_probe_a: 0,
                ..r
            },
            ReadinessParams {
                depth_probe_b: 0,
                ..r
            },
            ReadinessParams {
                max_depth_impact_bps: 0,
                ..r
            },
            ReadinessParams {
                max_depth_impact_bps: MAX_DEPTH_IMPACT_BPS + 1,
                ..r
            },
        ] {
            assert_eq!(bad.validate(), Err(AmmError::InvalidReadinessParams));
        }
    }

    #[test]
    fn mint_order() {
        let a = Pubkey::new_from_array([1; 32]);
        let b = Pubkey::new_from_array([2; 32]);
        assert_eq!(check_mint_order(&a, &b), Ok(()));
        assert_eq!(check_mint_order(&b, &a), Err(AmmError::MintsNotCanonical));
        assert_eq!(check_mint_order(&a, &a), Err(AmmError::MintsNotCanonical));
    }

    #[test]
    fn rebalancer_list_behind_the_timelock() {
        let mut c = config(MarketStatus::Open);
        let (admin, keeper) = (c.admin, c.keeper);
        let bot = Pubkey::new_from_array([9; 32]);
        assert!(c.is_rebalancer(&keeper), "keeper starts listed");
        assert!(!c.is_rebalancer(&bot));
        assert!(
            !c.is_rebalancer(&Pubkey::default()),
            "empty slot matches no one"
        );
        let none = Pubkey::default();
        assert_eq!(
            c.propose_rebalancers(&keeper, [bot, none, none, none], 0),
            Err(AmmError::Unauthorized)
        );
        assert_eq!(
            c.propose_rebalancers(&admin, [bot, none, bot, none], 0),
            Err(AmmError::InvalidRebalancers)
        );
        assert_eq!(
            c.apply_rebalancers(&admin, 0),
            Err(AmmError::NoPendingChange)
        );
        c.propose_rebalancers(&admin, [bot, keeper, none, none], 1_000)
            .unwrap();
        assert!(!c.is_rebalancer(&bot), "nothing changes before the delay");
        assert_eq!(
            c.apply_rebalancers(&admin, 1_000 + FEE_TIMELOCK_SECONDS - 1),
            Err(AmmError::TimelockNotElapsed)
        );
        assert_eq!(
            c.apply_rebalancers(&keeper, 1_000 + FEE_TIMELOCK_SECONDS),
            Err(AmmError::Unauthorized)
        );
        let old = c
            .apply_rebalancers(&admin, 1_000 + FEE_TIMELOCK_SECONDS)
            .unwrap();
        assert_eq!(old, [keeper, none, none, none]);
        assert!(c.is_rebalancer(&bot) && c.is_rebalancer(&keeper));
        // An all-empty list stops rebalancing; cancel drops a proposal.
        c.propose_rebalancers(&admin, [none; 4], 0).unwrap();
        c.cancel_rebalancers_change(&admin).unwrap();
        assert_eq!(
            c.cancel_rebalancers_change(&admin),
            Err(AmmError::NoPendingChange)
        );
        c.propose_rebalancers(&admin, [none; 4], 0).unwrap();
        c.apply_rebalancers(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        assert!(!c.is_rebalancer(&bot) && !c.is_rebalancer(&keeper));
    }
}
