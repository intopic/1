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
    pub reserved: [u8; 64],
}
