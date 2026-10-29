//! Fee changes behind a timelock (ADR-0004: delay for sensitive changes).
//!
//! - `update_fees` — admin proposes `fee_rate_ppm` / `protocol_share_bps`
//!   and the dynamic fee parameters (ADR-0005) within bounds; nothing
//!   changes for `FEE_TIMELOCK_SECONDS`.
//! - `apply_fees` — admin applies the pending change after the delay.
//! - `cancel_fee_change` — admin drops the pending change.
//! - `update_rebalancers` / `apply_rebalancers` / `cancel_rebalancers_change`
//!   — the same delay for the keys allowed to call `rebalance_pair`.
//!
//! `tick_spacing` is immutable. Changes apply to future swaps only; fees
//! already accrued are unaffected. Accounts: `admin` signer; `market` W.

use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_SEED;
use crate::errors::AmmError;
use crate::state::ProtocolConfig;

use crate::constants::MAX_REBALANCERS;
use crate::events::{
    FeeChangeProposed, FeesUpdated, RebalancersChangeProposed, RebalancersUpdated,
};
use crate::state::{DynamicFeeParams, MarketConfig};

#[derive(Accounts)]
pub struct UpdateFees<'info> {
    pub admin: Signer<'info>,
    #[account(mut)]
    pub market: Box<Account<'info, MarketConfig>>,
    /// Current protocol roles; the market's copies must match (D-04).
    #[account(
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.roles_version == market.roles_version @ AmmError::StaleRoles,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
}

pub fn handle_update_fees(
    ctx: Context<UpdateFees>,
    fee_rate_ppm: u32,
    protocol_share_bps: u16,
    dynamic: DynamicFeeParams,
) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let pending =
        ctx.accounts
            .market
            .propose_fees(&by, fee_rate_ppm, protocol_share_bps, dynamic, now)?;
    emit!(FeeChangeProposed {
        market: ctx.accounts.market.key(),
        fee_rate_ppm,
        protocol_share_bps,
        dynamic,
        eta: pending.eta,
        by,
    });
    Ok(())
}

pub fn handle_apply_fees(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let old_dynamic = ctx.accounts.market.dynamic_fees;
    let (old, new) = ctx.accounts.market.apply_fees(&by, now)?;
    emit!(FeesUpdated {
        market: ctx.accounts.market.key(),
        old,
        new,
        old_dynamic,
        new_dynamic: ctx.accounts.market.dynamic_fees,
        by
    });
    Ok(())
}

pub fn handle_cancel_fee_change(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    ctx.accounts.market.cancel_fee_change(&by)?;
    Ok(())
}

pub fn handle_update_rebalancers(
    ctx: Context<UpdateFees>,
    keys: [Pubkey; MAX_REBALANCERS],
) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let pending = ctx.accounts.market.propose_rebalancers(&by, keys, now)?;
    emit!(RebalancersChangeProposed {
        market: ctx.accounts.market.key(),
        keys,
        eta: pending.eta,
        by,
    });
    Ok(())
}

pub fn handle_apply_rebalancers(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    let now = Clock::get()?.unix_timestamp;
    let old = ctx.accounts.market.apply_rebalancers(&by, now)?;
    emit!(RebalancersUpdated {
        market: ctx.accounts.market.key(),
        old,
        new: ctx.accounts.market.rebalancers,
        by,
    });
    Ok(())
}

pub fn handle_cancel_rebalancers_change(ctx: Context<UpdateFees>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    ctx.accounts.market.cancel_rebalancers_change(&by)?;
    Ok(())
}
