//! `set_paused` — `Open -> Paused` (pauser or admin), `Paused -> Open` (admin).
//! `set_pause_flags` — selective pauses (ADR-0006): the pauser may only add
//! bits, the admin may set any; unknown bits are rejected.
//!
//! Accounts: `authority` signer; `market` W. Moves no funds. A paused market
//! rejects swaps and new liquidity; LP withdraw/collect stay available
//! (ADR-0002).

use anchor_lang::prelude::*;

use crate::events::{MarketPauseChanged, PauseFlagsChanged};
use crate::state::{MarketConfig, MarketStatus};

#[derive(Accounts)]
pub struct SetPaused<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: Account<'info, MarketConfig>,
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
