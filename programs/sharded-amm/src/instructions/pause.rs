//! `set_paused` — `Open -> Paused` (pauser or admin), `Paused -> Open` (admin).
//! `set_pause_flags` — selective pauses (ADR-0006): the pauser may only add
//! bits, the admin may set any; unknown bits are rejected.
//!
//! `set_shard_pause_flags` — the same flags on one shard only, with the same
//! rules: a problem on one shard does not stop the others. It cannot lift
//! `PAUSE_SWAPS`; that is `reopen_shard`.
//! `reopen_shard` — admin only: lifts `PAUSE_SWAPS` (and sets the other bits
//! to `flags`) only if, in the same transaction, the shard's price is within
//! `readiness.max_activation_deviation_bps` of the median price of **every**
//! other active shard that is trading (D-09; remaining accounts, read-only,
//! exactly those shards, each once). If none trades, it is a bootstrap
//! against every other active shard (all swap-paused), which checks
//! consistency among the shards but not against outside markets. Otherwise
//! it fails and the shard stays paused.
//! The registry's `swap_paused_mask` mirrors which shards are swap-paused.
//!
//! Accounts: `authority` signer; `market` W (R for the shard variant);
//! `shard` W (`has_one = market`). Moves no funds. A paused market
//! rejects swaps and new liquidity; LP withdraw/collect stay available
//! (ADR-0002).

use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_SEED;
use crate::state::ProtocolConfig;

use crate::constants::{PAUSE_SWAPS, REGISTRY_SEED};
use crate::errors::AmmError;
use crate::events::{MarketPauseChanged, PauseFlagsChanged, ShardPauseFlagsChanged, ShardReopened};
use crate::state::{MarketConfig, MarketStatus, ShardRegistry, ShardState, ShardStatus};

use super::market_lifecycle::{median_of, within_deviation};

#[derive(Accounts)]
pub struct SetPaused<'info> {
    pub authority: Signer<'info>,
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

#[derive(Accounts)]
pub struct SetShardPauseFlags<'info> {
    pub authority: Signer<'info>,
    pub market: Box<Account<'info, MarketConfig>>,
    /// Current protocol roles; the market's copies must match (D-04).
    #[account(
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.roles_version == market.roles_version @ AmmError::StaleRoles,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(mut, seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Box<Account<'info, ShardRegistry>>,
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
    let bit = 1u64 << ctx.accounts.shard.id;
    if flags & PAUSE_SWAPS != 0 {
        ctx.accounts.registry.swap_paused_mask |= bit;
    }
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
    /// Current protocol roles; the market's copies must match (D-04).
    #[account(
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.roles_version == market.roles_version @ AmmError::StaleRoles,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(mut, seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
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
    // References: every trading shard; if none trades (every other shard
    // swap-paused, re-audit P2-1), a bootstrap against all other active
    // shards, so recovery never depends on shards that cannot reopen
    // themselves. The caller cannot choose a subset (D-09).
    let open = acc.registry.open_others(shard.id);
    let bootstrap = open == 0;
    let mask = if bootstrap {
        acc.registry.active_mask & !(1u64 << shard.id)
    } else {
        open
    };
    // No other active shard at all (a one-shard market): nothing to compare.
    let median = median_of(
        &acc.market.key(),
        &acc.registry,
        mask,
        ctx.remaining_accounts,
    )?
    .unwrap_or(shard.sqrt_price_x64);
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
    let bit = 1u64 << ctx.accounts.shard.id;
    ctx.accounts.registry.swap_paused_mask &= !bit;
    emit!(ShardReopened {
        market: ctx.accounts.market.key(),
        shard: ctx.accounts.shard.key(),
        shard_id: ctx.accounts.shard.id,
        old,
        new: flags,
        sqrt_price_x64: ctx.accounts.shard.sqrt_price_x64,
        median_sqrt_price_x64: median,
        references: mask.count_ones() as u8,
        bootstrap,
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
