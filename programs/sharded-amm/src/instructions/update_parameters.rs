//! Fee changes behind a timelock (ADR-0004: delay for sensitive changes).
//!
//! - `update_fees` — admin proposes `fee_rate_ppm` / `protocol_share_bps`
//!   within bounds; nothing changes for `FEE_TIMELOCK_SECONDS`.
//! - `apply_fees` — admin applies the pending change after the delay.
//! - `cancel_fee_change` — admin drops the pending change.
//!
//! `tick_spacing` is immutable. Changes apply to future swaps only; fees
//! already accrued are unaffected. Accounts: `admin` signer; `market` W.

use anchor_lang::prelude::*;

use crate::events::{FeeChangeProposed, FeesUpdated};
use crate::state::MarketConfig;

#[derive(Accounts)]
pub struct UpdateFees<'info> {
    pub admin: Signer<'info>,
    #[account(mut)]
    pub market: Account<'info, MarketConfig>,
}

pub fn handle_update_fees(
    ctx: Context<UpdateFees>,
    fee_rate_ppm: u32,
    protocol_share_bps: u16,
) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let pending = ctx
        .accounts
        .market
        .propose_fees(&by, fee_rate_ppm, protocol_share_bps, now)?;
    emit!(FeeChangeProposed {
        market: ctx.accounts.market.key(),
        fee_rate_ppm,
        protocol_share_bps,
        eta: pending.eta,
        by,
    });
    Ok(())
}

pub fn handle_apply_fees(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let (old, new) = ctx.accounts.market.apply_fees(&by, now)?;
    emit!(FeesUpdated {
        market: ctx.accounts.market.key(),
        old,
        new,
        by
    });
    Ok(())
}

pub fn handle_cancel_fee_change(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    ctx.accounts.market.cancel_fee_change(&by)?;
    Ok(())
}
