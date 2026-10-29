//! `ShardState`: one independent CLMM. The only market-level state written
//! by swaps and LP operations, together with its own two vaults.

use anchor_lang::prelude::*;

use crate::state::ShardStatus;

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct ShardState {
    pub market: Pubkey,
    pub id: u8,
    /// Always 0 in v1; part of the PDA so a future reuse cannot alias claims.
    pub generation: u64,
    pub status: ShardStatus,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub vault_authority_bump: u8,
    /// Q64.64. Zero until the price is initialized at seeding (ENG-04/05).
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub liquidity: u128,
    /// Q128 LP fee per unit of liquidity as little-endian U256 limbs.
    /// Monotonic and checked, never wraps (ADR-0001, `math::fee_math`).
    pub fee_growth_global_a_x128: [u64; 4],
    pub fee_growth_global_b_x128: [u64; 4],
    /// Accrued locally; moved only by `collect_protocol_fees`.
    pub protocol_fees_a: u64,
    pub protocol_fees_b: u64,
    pub bump: u8,
    /// Per-shard dynamic fee state (ADR-0005); written only by swaps.
    pub dynamic_fee: DynamicFeeState,
    pub reserved: [u8; 34],
}

/// Volatility and load tracker of one shard (ADR-0005). Lives in the shard,
/// never in the market, so a swap writes no account another shard reads.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct DynamicFeeState {
    /// Volatility after the last swap, in `VOL_UNIT` per `tick_spacing`.
    pub vol_accumulator: u32,
    /// Part carried over from the previous episode.
    pub vol_reference: u32,
    /// Tick at which the current episode started.
    pub tick_reference: i32,
    /// Slot at which the current episode started.
    pub reference_slot: u64,
    pub last_update_slot: u64,
    /// Swaps executed in `last_update_slot`.
    pub slot_swaps: u16,
}
