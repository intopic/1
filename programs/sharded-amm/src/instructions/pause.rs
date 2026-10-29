//! `set_paused` — `Open -> Paused` (pauser or admin), `Paused -> Open` (admin).
//! `set_pause_flags` — selective pauses (ADR-0006): the pauser may only add
//! bits, the admin may set any; unknown bits are rejected.
//!
//! `set_shard_pause_flags` — the same flags on one shard only, with the same
//! rules: a problem on one shard does not stop the others. It cannot lift
//! `PAUSE_SWAPS`; that is `reopen_shard`.
//! `reopen_shard` — admin only: lifts `PAUSE_SWAPS` (and sets the other bits
//! to `flags`) only if, in the same transaction, the shard's price is within
//! `readiness.max_activation_deviation_bps` of the median price of at least
//! `MIN_REOPEN_REFERENCES` other active shards that are trading (remaining
//! accounts, read-only). Otherwise it fails and the shard stays paused.
//!
//! Accounts: `authority` signer; `market` W (R for the shard variant);
//! `shard` W (`has_one = market`). Moves no funds. A paused market
//! rejects swaps and new liquidity; LP withdraw/collect stay available
//! (ADR-0002).

use anchor_lang::prelude::*;

use crate::constants::{MIN_REOPEN_REFERENCES, PAUSE_SWAPS, REGISTRY_SEED};
use crate::errors::AmmError;
use crate::events::{MarketPauseChanged, PauseFlagsChanged, ShardPauseFlagsChanged, ShardReopened};
use crate::state::{MarketConfig, MarketStatus, ShardRegistry, ShardState, ShardStatus};

use super::market_lifecycle::within_deviation;

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
    require!(
        old & PAUSE_SWAPS == 0 || flags & PAUSE_SWAPS != 0,
        AmmError::ReopenNeedsPriceCheck
    );
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

#[derive(Accounts)]
pub struct ReopenShard<'info> {
    pub admin: Signer<'info>,
    #[account(has_one = admin)]
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Box<Account<'info, ShardRegistry>>,
    #[account(mut, has_one = market)]
    pub shard: Box<Account<'info, ShardState>>,
}

pub fn handle_reopen_shard<'info>(
    ctx: Context<'info, ReopenShard<'info>>,
    flags: u8,
) -> Result<()> {
    let acc = &ctx.accounts;
    let shard = &acc.shard;
    require!(shard.swaps_paused(), AmmError::NoPendingChange);
    require!(flags & PAUSE_SWAPS == 0, AmmError::InvalidPauseFlags);
    require!(
        shard.status == ShardStatus::Active && acc.registry.is_active(shard.id),
        AmmError::ShardNotActive
    );
    acc.market
        .check_pause_change(&acc.admin.key(), shard.pause_flags, flags)?;
    require!(
        ctx.remaining_accounts.len() >= MIN_REOPEN_REFERENCES,
        AmmError::InvalidAccountCount
    );
    let mut ids = 0u64;
    let mut prices = Vec::with_capacity(ctx.remaining_accounts.len());
    for ai in ctx.remaining_accounts {
        let r: Account<ShardState> = Account::try_from(ai)?;
        require!(r.market == acc.market.key(), AmmError::ShardMismatch);
        require!(
            r.id != shard.id && ids & (1 << r.id) == 0,
            AmmError::DuplicateAccount
        );
        ids |= 1 << r.id;
        require!(
            r.status == ShardStatus::Active && acc.registry.is_active(r.id),
            AmmError::ShardNotActive
        );
        require!(!r.swaps_paused(), AmmError::ShardPaused);
        prices.push(r.sqrt_price_x64);
    }
    prices.sort_unstable();
    let median = prices[prices.len() / 2];
    require!(
        within_deviation(
            shard.sqrt_price_x64,
            median,
            acc.market.readiness.max_activation_deviation_bps
        ),
        AmmError::PriceDeviationTooHigh
    );
    let old = shard.pause_flags;
    ctx.accounts.shard.pause_flags = flags;
    emit!(ShardReopened {
        market: ctx.accounts.market.key(),
        shard: ctx.accounts.shard.key(),
        shard_id: ctx.accounts.shard.id,
        old,
        new: flags,
        sqrt_price_x64: ctx.accounts.shard.sqrt_price_x64,
        median_sqrt_price_x64: median,
        references: prices.len() as u8,
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
