//! `update_fees` — admin changes `fee_rate_ppm` / `protocol_share_bps` within
//! bounds. `tick_spacing` is immutable. Applies to future swaps only; fees
//! already accrued are unaffected.
//!
//! Accounts: `admin` signer; `market` W.
//! Not yet implemented: a timelock for sensitive changes (ADR-0004).

use anchor_lang::prelude::*;

use crate::events::FeesUpdated;
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
    let old = ctx
        .accounts
        .market
        .update_fees(&by, fee_rate_ppm, protocol_share_bps)?;
    emit!(FeesUpdated {
        market: ctx.accounts.market.key(),
        old,
        new: ctx.accounts.market.params,
        by,
    });
    Ok(())
}
