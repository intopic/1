//! `set_paused` — `Open -> Paused` (pauser or admin), `Paused -> Open` (admin).
//! `set_pause_flags` — selective pauses (ADR-0006): the pauser may only add
//! bits, the admin may set any; unknown bits are rejected.
//!
//! `set_shard_pause_flags` — the same flags on one shard only, with the same
//! rules: a problem on one shard does not stop the others.
//!
//! Accounts: `authority` signer; `market` W (R for the shard variant);
//! `shard` W (`has_one = market`). Moves no funds. A paused market
//! rejects swaps and new liquidity; LP withdraw/collect stay available
//! (ADR-0002).

use anchor_lang::prelude::*;

use crate::events::{MarketPauseChanged, PauseFlagsChanged, ShardPauseFlagsChanged};
use crate::state::{MarketConfig, MarketStatus, ShardState};

#[derive(Accounts)]
pub struct SetPaused<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: Box<Account<'info, MarketConfig>>,
}

#[derive(Accounts)]
pub struct SetShardPauseFlags<'info> {
    pub authority: Signer<'info>,
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(mut, has_one = market)]
    pub shard: Box<Account<'info, ShardState>>,
}

pub fn handle_set_shard_pause_flags(ctx: Context<SetShardPauseFlags>, flags: u8) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let old = ctx.accounts.shard.pause_flags;
    ctx.accounts.market.check_pause_change(&by, old, flags)?;
    ctx.accounts.shard.pause_flags = flags;
    emit!(ShardPauseFlagsChanged {
        market: ctx.accounts.market.key(),
        shard: ctx.accounts.shard.key(),
        shard_id: ctx.accounts.shard.id,
        old,
        new: flags,
        by,
    });
    Ok(())
}

pub fn handle_set_pause_flags(ctx: Context<SetPaused>, flags: u8) -> Result<()> {
    let by = ctx.accounts.authority.key();
    let old = ctx.accounts.market.set_pause_flags(&by, flags)?;
    emit!(PauseFlagsChanged {
        market: ctx.accounts.market.key(),
        old,
        new: flags,
        by,
    });
    Ok(())
}

pub fn handle_set_paused(ctx: Context<SetPaused>, paused: bool) -> Result<()> {
    let by = ctx.accounts.authority.key();
    ctx.accounts.market.set_paused(&by, paused)?;
    emit!(MarketPauseChanged {
        market: ctx.accounts.market.key(),
        paused: ctx.accounts.market.status == MarketStatus::Paused,
        by,
    });
    Ok(())
}
