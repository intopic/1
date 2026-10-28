//! Sharded-tick CLMM for Solana: one market, 8 initial shards, capacity 56.
//!
//! Status: ENG-03 in progress. Implemented: constants, shard bitmaps, registry
//! lifecycle, `initialize_market`, `prepare_shard` (accounts, PDAs, vaults),
//! `set_paused` and `update_fees`.
//! No liquidity, swap or fee handlers exist yet; see STATUS.md. Not audited.
#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

use instructions::*;

// Placeholder program ID for local builds. The deploy keypair is not in the
// repository; a devnet/mainnet ID is assigned at release (ENG-10).
declare_id!("B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy");

#[program]
pub mod sharded_amm {
    use super::*;

    pub fn initialize_market(
        ctx: Context<InitializeMarket>,
        args: InitializeMarketArgs,
    ) -> Result<()> {
        handle_initialize_market(ctx, args)
    }

    pub fn prepare_shard(ctx: Context<PrepareShard>, id: u8, generation: u64) -> Result<()> {
        handle_prepare_shard(ctx, id, generation)
    }

    pub fn set_paused(ctx: Context<SetPaused>, paused: bool) -> Result<()> {
        handle_set_paused(ctx, paused)
    }

    pub fn update_fees(
        ctx: Context<UpdateFees>,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
    ) -> Result<()> {
        handle_update_fees(ctx, fee_rate_ppm, protocol_share_bps)
    }
}
