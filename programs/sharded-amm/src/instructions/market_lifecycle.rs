//! Opening, cancelling and expanding a market.
//!
//! - `open_market` — keeper or admin. Remaining accounts: for each shard
//!   `0..8` in order, `[shard (W), vault_a, vault_b]`. Every shard is
//!   re-checked in this transaction (identity, price, liquidity at the
//!   current price, real vault balances against `MarketConfig::readiness`);
//!   then all eight become `Active` and the market `Open` atomically.
//! - `cancel_setup` — admin, only while `Initializing`. The market becomes
//!   `Cancelled`: no new liquidity, prepares or swaps; every LP can still
//!   `decrease_liquidity` and `collect_fees` (the refund path).
//! - `activate_shard` — keeper, market `Open`. Activates one prepared shard
//!   (ID 8..56) after the same readiness checks plus a price check against
//!   the median of **every** open shard (D-09): remaining accounts are
//!   exactly the active, trading shards, each once, read-only. The caller
//!   cannot pick a convenient subset.

use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_SEED;
use crate::state::ProtocolConfig;
use anchor_spl::token::TokenAccount;

use crate::constants::{INITIAL_SHARDS, REGISTRY_SEED};
use crate::errors::AmmError;
use crate::events::{MarketCancelled, MarketOpened, ShardActivated};
use crate::math::wide::U256;
use crate::state::{MarketConfig, ReadinessParams, ShardRegistry, ShardState, ShardStatus};

/// Readiness of one shard against its real vault accounts.
fn shard_ready(
    shard: &ShardState,
    market: &Pubkey,
    config_version: u32,
    vault_a: &Account<TokenAccount>,
    vault_b: &Account<TokenAccount>,
    r: &ReadinessParams,
) -> Result<bool> {
    require!(shard.market == *market, AmmError::ShardMismatch);
    require!(
        vault_a.key() == shard.vault_a && vault_b.key() == shard.vault_b,
        AmmError::InvalidVault
    );
    Ok(shard.sqrt_price_x64 != 0
        && shard.depth_verified
        && shard.depth_config_version == config_version
        && r.is_ready(shard.liquidity, vault_a.amount, vault_b.amount))
}

#[derive(Accounts)]
pub struct OpenMarket<'info> {
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
    #[account(mut, seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Account<'info, ShardRegistry>,
}

pub fn handle_open_market<'info>(ctx: Context<'info, OpenMarket<'info>>) -> Result<()> {
    let market_key = ctx.accounts.market.key();
    let m = &ctx.accounts.market;
    require!(
        m.can_prepare(&ctx.accounts.authority.key()),
        AmmError::Unauthorized
    );
    let rem = ctx.remaining_accounts;
    require!(
        rem.len() == 3 * INITIAL_SHARDS as usize,
        AmmError::InvalidAccountCount
    );
    let mut ready_mask = 0u64;
    let mut shards = Vec::with_capacity(INITIAL_SHARDS as usize);
    for id in 0..INITIAL_SHARDS {
        let i = 3 * id as usize;
        let shard: Account<ShardState> = Account::try_from(&rem[i])?;
        let vault_a: Account<TokenAccount> = Account::try_from(&rem[i + 1])?;
        let vault_b: Account<TokenAccount> = Account::try_from(&rem[i + 2])?;
        require!(rem[i].is_writable, AmmError::ShardMismatch);
        require!(
            shard.id == id && shard.status == ShardStatus::Prepared,
            AmmError::ShardMismatch
        );
        if shard_ready(
            &shard,
            &market_key,
            m.config_version,
            &vault_a,
            &vault_b,
            &m.readiness,
        )? {
            ready_mask |= 1 << id;
        }
        shards.push(shard);
    }
    let mut status = m.status;
    ctx.accounts
        .registry
        .finalize_initial(&mut status, ready_mask)?;
    ctx.accounts.market.status = status;
    for mut shard in shards {
        shard.status = ShardStatus::Active;
        shard.exit(&crate::ID)?;
    }
    emit!(MarketOpened {
        market: market_key,
        active_mask: ctx.accounts.registry.active_mask,
        registry_version: ctx.accounts.registry.registry_version,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct CancelSetup<'info> {
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

pub fn handle_cancel_setup(ctx: Context<CancelSetup>) -> Result<()> {
    let by = ctx.accounts.admin.key();
    ctx.accounts.market.cancel_setup(&by)?;
    emit!(MarketCancelled {
        market: ctx.accounts.market.key(),
        by
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ActivateShard<'info> {
    pub keeper: Signer<'info>,
    pub market: Box<Account<'info, MarketConfig>>,
    /// Current protocol roles; the market's copies must match (D-04).
    #[account(
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.roles_version == market.roles_version @ AmmError::StaleRoles,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(mut, seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Account<'info, ShardRegistry>,
    #[account(mut, has_one = market, has_one = vault_a, has_one = vault_b)]
    pub shard: Box<Account<'info, ShardState>>,
    pub vault_a: Box<Account<'info, TokenAccount>>,
    pub vault_b: Box<Account<'info, TokenAccount>>,
}

/// Median `sqrt_price_x64` of exactly the shards in `mask`: `accounts` must
/// hold each of them once (any order) and nothing else, all active shards
/// of `market`. For an even count it is the upper median. `None` if `mask`
/// is empty.
pub fn median_of<'info>(
    market: &Pubkey,
    registry: &ShardRegistry,
    mask: u64,
    accounts: &'info [AccountInfo<'info>],
) -> Result<Option<u128>> {
    require!(
        accounts.len() == mask.count_ones() as usize,
        AmmError::InvalidAccountCount
    );
    let mut seen = 0u64;
    let mut prices = Vec::with_capacity(accounts.len());
    for ai in accounts {
        let r: Account<ShardState> = Account::try_from(ai)?;
        require!(r.market == *market, AmmError::ShardMismatch);
        let bit = 1u64 << r.id;
        require!(seen & bit == 0, AmmError::DuplicateAccount);
        require!(mask & bit != 0, AmmError::ShardMismatch);
        require!(
            r.status == ShardStatus::Active && registry.is_active(r.id),
            AmmError::ShardNotActive
        );
        seen |= bit;
        prices.push(r.sqrt_price_x64);
    }
    prices.sort_unstable();
    Ok(prices.get(prices.len() / 2).copied())
}

/// `|p / p_ref - 1| <= bps` in both directions, with p = s^2, exactly.
pub fn within_deviation(s: u128, s_ref: u128, bps: u16) -> bool {
    let (p, q) = (
        U256::from(s) * U256::from(s),
        U256::from(s_ref) * U256::from(s_ref),
    );
    let (lo, hi) = (
        U256::from(10_000u32),
        U256::from(10_000u32 + u32::from(bps)),
    );
    p * lo <= q * hi && q * lo <= p * hi
}

pub fn handle_activate_shard<'info>(ctx: Context<'info, ActivateShard<'info>>) -> Result<()> {
    let m = &ctx.accounts.market;
    require!(
        ctx.accounts.keeper.key() == m.keeper,
        AmmError::Unauthorized
    );
    let shard = &ctx.accounts.shard;
    require!(
        shard.status == ShardStatus::Prepared,
        AmmError::ShardAlreadyActive
    );
    let open = ctx.accounts.registry.open_others(shard.id);
    require!(open != 0, AmmError::ShardNotActive);
    let median = median_of(
        &m.key(),
        &ctx.accounts.registry,
        open,
        ctx.remaining_accounts,
    )?
    .ok_or(AmmError::ShardNotActive)?;
    let ready = shard_ready(
        shard,
        &m.key(),
        m.config_version,
        &ctx.accounts.vault_a,
        &ctx.accounts.vault_b,
        &m.readiness,
    )?;
    require!(
        within_deviation(
            shard.sqrt_price_x64,
            median,
            m.readiness.max_activation_deviation_bps
        ),
        AmmError::PriceDeviationTooHigh
    );
    ctx.accounts.registry.activate(m.status, shard.id, ready)?;
    ctx.accounts.shard.status = ShardStatus::Active;
    emit!(ShardActivated {
        market: m.key(),
        shard: ctx.accounts.shard.key(),
        id: ctx.accounts.shard.id,
        active_mask: ctx.accounts.registry.active_mask,
        registry_version: ctx.accounts.registry.registry_version,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deviation_is_symmetric_and_exact() {
        let s = 1u128 << 64;
        assert!(within_deviation(s, s, 0));
        // sqrt(1.01) * 2^64 is about 1.00498756 * 2^64: just inside 100 bps.
        let inside = s + s / 10_000 * 49;
        assert!(within_deviation(inside, s, 100) && within_deviation(s, inside, 100));
        let outside = s + s / 10_000 * 51;
        assert!(!within_deviation(outside, s, 100) && !within_deviation(s, outside, 100));
        assert!(!within_deviation(s + 1, s, 0));
    }
}
