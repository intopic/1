//! Sharded-tick CLMM for Solana: one market, 8 initial shards, capacity 56.
//!
//! Instructions: market setup (`initialize_market`, `prepare_shard`,
//! `initialize_price`, `open_market`, `cancel_setup`, `activate_shard`),
//! liquidity (`init_tick_array`, `open_position`, `increase_liquidity`,
//! `decrease_liquidity`, `collect_fees`, `close_position`), trading
//! (`swap_exact_in`) and controls (`set_paused`, fee timelock,
//! `collect_protocol_fees`). See STATUS.md. Not audited.
#![allow(unexpected_cfgs)]

use anchor_lang::prelude::*;

pub mod constants;
pub mod engine;
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

    /// Selective pauses for swaps, new liquidity and rebalancing (ADR-0006).
    pub fn set_pause_flags(ctx: Context<SetPaused>, flags: u8) -> Result<()> {
        handle_set_pause_flags(ctx, flags)
    }

    /// Proposes a fee change; applied by `apply_fees` after the timelock.
    pub fn update_fees(
        ctx: Context<UpdateFees>,
        fee_rate_ppm: u32,
        protocol_share_bps: u16,
        dynamic: state::DynamicFeeParams,
    ) -> Result<()> {
        handle_update_fees(ctx, fee_rate_ppm, protocol_share_bps, dynamic)
    }

    pub fn apply_fees(ctx: Context<UpdateFees>) -> Result<()> {
        handle_apply_fees(ctx)
    }

    pub fn cancel_fee_change(ctx: Context<UpdateFees>) -> Result<()> {
        handle_cancel_fee_change(ctx)
    }

    /// Proposes the keys allowed to call `rebalance_pair`; applied by
    /// `apply_rebalancers` after the timelock.
    pub fn update_rebalancers(
        ctx: Context<UpdateFees>,
        keys: [Pubkey; constants::MAX_REBALANCERS],
    ) -> Result<()> {
        handle_update_rebalancers(ctx, keys)
    }

    pub fn apply_rebalancers(ctx: Context<UpdateFees>) -> Result<()> {
        handle_apply_rebalancers(ctx)
    }

    pub fn cancel_rebalancers_change(ctx: Context<UpdateFees>) -> Result<()> {
        handle_cancel_rebalancers_change(ctx)
    }

    pub fn initialize_price(ctx: Context<InitializePrice>, sqrt_price_x64: u128) -> Result<()> {
        handle_initialize_price(ctx, sqrt_price_x64)
    }

    pub fn init_tick_array(ctx: Context<InitTickArray>, start_tick_index: i32) -> Result<()> {
        handle_init_tick_array(ctx, start_tick_index)
    }

    pub fn open_position(
        ctx: Context<OpenPosition>,
        tick_lower: i32,
        tick_upper: i32,
    ) -> Result<()> {
        handle_open_position(ctx, tick_lower, tick_upper)
    }

    pub fn increase_liquidity<'info>(
        ctx: Context<'info, ModifyLiquidity<'info>>,
        liquidity: u128,
        amount_a_max: u64,
        amount_b_max: u64,
    ) -> Result<()> {
        handle_increase_liquidity(ctx, liquidity, amount_a_max, amount_b_max)
    }

    pub fn decrease_liquidity<'info>(
        ctx: Context<'info, ModifyLiquidity<'info>>,
        liquidity: u128,
        amount_a_min: u64,
        amount_b_min: u64,
    ) -> Result<()> {
        handle_decrease_liquidity(ctx, liquidity, amount_a_min, amount_b_min)
    }

    pub fn collect_fees<'info>(ctx: Context<'info, ModifyLiquidity<'info>>) -> Result<()> {
        handle_collect_fees(ctx)
    }

    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        handle_close_position(ctx)
    }

    pub fn open_market<'info>(ctx: Context<'info, OpenMarket<'info>>) -> Result<()> {
        handle_open_market(ctx)
    }

    pub fn cancel_setup(ctx: Context<CancelSetup>) -> Result<()> {
        handle_cancel_setup(ctx)
    }

    pub fn activate_shard(ctx: Context<ActivateShard>) -> Result<()> {
        handle_activate_shard(ctx)
    }

    pub fn swap_exact_in<'info>(
        ctx: Context<'info, SwapExactIn<'info>>,
        args: SwapArgs,
    ) -> Result<()> {
        handle_swap_exact_in(ctx, args)
    }

    /// Capital-free rebalancing between two shards, by the listed rebalancers only (ADR-0006).
    pub fn rebalance_pair<'info>(
        ctx: Context<'info, RebalancePair<'info>>,
        args: RebalanceArgs,
    ) -> Result<()> {
        handle_rebalance_pair(ctx, args)
    }

    pub fn collect_protocol_fees(ctx: Context<CollectProtocolFees>) -> Result<()> {
        handle_collect_protocol_fees(ctx)
    }
}
