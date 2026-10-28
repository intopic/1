//! `initialize_price` — sets a prepared shard's starting sqrt price once.
//!
//! Accounts: `authority` signer (keeper, or admin while `Initializing`);
//! `market` R; `shard` W (`has_one = market`). The price must be inside
//! `[MIN_SQRT_PRICE_X64, MAX_SQRT_PRICE_X64)`. Liquidity can be added only
//! after this. No funds move.

use anchor_lang::prelude::*;

use crate::engine::Pool;
use crate::errors::AmmError;
use crate::events::PriceInitialized;
use crate::state::{MarketConfig, MarketStatus, ShardState, ShardStatus};

#[derive(Accounts)]
pub struct InitializePrice<'info> {
    pub authority: Signer<'info>,
    pub market: Account<'info, MarketConfig>,
    #[account(mut, has_one = market)]
    pub shard: Account<'info, ShardState>,
}

pub fn handle_initialize_price(ctx: Context<InitializePrice>, sqrt_price_x64: u128) -> Result<()> {
    let market = &ctx.accounts.market;
    require!(
        market.can_prepare(&ctx.accounts.authority.key()),
        AmmError::Unauthorized
    );
    require!(
        matches!(
            market.status,
            MarketStatus::Initializing | MarketStatus::Open
        ),
        AmmError::InvalidMarketStatus
    );
    let shard = &mut ctx.accounts.shard;
    require!(
        shard.status == ShardStatus::Prepared,
        AmmError::ShardAlreadyActive
    );
    require!(shard.sqrt_price_x64 == 0, AmmError::PriceAlreadyInitialized);
    let pool = Pool::at_price(sqrt_price_x64)?;
    pool.store(shard);
    emit!(PriceInitialized {
        shard: shard.key(),
        sqrt_price_x64,
        tick: pool.tick
    });
    Ok(())
}
