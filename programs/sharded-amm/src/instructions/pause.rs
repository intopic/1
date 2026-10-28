//! `set_paused` — `Open -> Paused` (pauser or admin), `Paused -> Open` (admin).
//!
//! Accounts: `authority` signer; `market` W. Moves no funds. A paused market
//! rejects swaps and new liquidity; LP withdraw/collect stay available
//! (ADR-0002).

use anchor_lang::prelude::*;

use crate::events::MarketPauseChanged;
use crate::state::{MarketConfig, MarketStatus};

#[derive(Accounts)]
pub struct SetPaused<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: Account<'info, MarketConfig>,
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
