//! `init_tick_array` — permissionless creation of one tick array.
//!
//! Accounts: `payer` W signer; `market` R; `shard` R (`has_one = market`);
//! `tick_array` W init, `["ticks", shard, start_i32_le]`; `system_program`.
//! The start must be a multiple of `32 * tick_spacing` and cover at least
//! one tick inside the domain.

use anchor_lang::prelude::*;

use crate::constants::TICK_ARRAY_SEED;
use crate::state::{check_array_start, MarketConfig, ShardState, TickArray};

#[derive(Accounts)]
#[instruction(start_tick_index: i32)]
pub struct InitTickArray<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub market: Account<'info, MarketConfig>,
    #[account(has_one = market)]
    pub shard: Account<'info, ShardState>,
    #[account(
        init,
        payer = payer,
        space = 8 + std::mem::size_of::<TickArray>(),
        seeds = [TICK_ARRAY_SEED, shard.key().as_ref(), &start_tick_index.to_le_bytes()],
        bump,
    )]
    pub tick_array: AccountLoader<'info, TickArray>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_tick_array(ctx: Context<InitTickArray>, start_tick_index: i32) -> Result<()> {
    check_array_start(start_tick_index, ctx.accounts.market.params.tick_spacing)?;
    let mut a = ctx.accounts.tick_array.load_init()?;
    *a = TickArray::new(
        ctx.accounts.shard.key(),
        start_tick_index,
        ctx.bumps.tick_array,
    );
    Ok(())
}
