//! `initialize_market` — admin creates `MarketConfig` + empty `ShardRegistry`.
//!
//! Accounts (R/W/signer):
//! - `admin`          W, signer — payer; becomes `MarketConfig::admin`
//! - `mint_a/mint_b`  R — classic SPL Token mints, canonical order, distinct
//! - `market`         W, init — `["market", schema_u16_le, mint_a, mint_b, market_id_u64_le]`
//! - `registry`       W, init — `["registry", market]`
//! - `token_program`  R — pinned to SPL Token; Token-2022 mints fail the owner check
//! - `system_program` R
//!
//! Post: status `Initializing`, both masks zero, version 0. No funds move.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};

use crate::constants::{MARKET_SEED, REGISTRY_SEED, SCHEMA_VERSION};
use crate::errors::AmmError;
use crate::events::MarketInitialized;
use crate::state::{
    check_mint_order, DynamicFeeParams, MarketConfig, MarketParams, MarketStatus, PendingFees,
    PendingRebalancers, ReadinessParams, ShardRegistry,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeMarketArgs {
    pub market_id: u64,
    pub params: MarketParams,
    pub keeper: Pubkey,
    pub revenue_authority: Pubkey,
    pub pauser: Pubkey,
    pub readiness: ReadinessParams,
}

#[derive(Accounts)]
#[instruction(args: InitializeMarketArgs)]
pub struct InitializeMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    pub mint_a: Account<'info, Mint>,
    pub mint_b: Account<'info, Mint>,
    #[account(
        init,
        payer = admin,
        space = 8 + MarketConfig::INIT_SPACE,
        seeds = [
            MARKET_SEED,
            &SCHEMA_VERSION.to_le_bytes(),
            mint_a.key().as_ref(),
            mint_b.key().as_ref(),
            &args.market_id.to_le_bytes(),
        ],
        bump,
    )]
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(
        init,
        payer = admin,
        space = 8 + ShardRegistry::INIT_SPACE,
        seeds = [REGISTRY_SEED, market.key().as_ref()],
        bump,
    )]
    pub registry: Account<'info, ShardRegistry>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_market(
    ctx: Context<InitializeMarket>,
    args: InitializeMarketArgs,
) -> Result<()> {
    check_mint_order(&ctx.accounts.mint_a.key(), &ctx.accounts.mint_b.key())?;
    args.params.validate()?;
    args.readiness.validate()?;
    for role in [&args.keeper, &args.revenue_authority, &args.pauser] {
        require!(*role != Pubkey::default(), AmmError::Unauthorized);
    }

    let market_key = ctx.accounts.market.key();
    ctx.accounts.market.set_inner(MarketConfig {
        schema_version: SCHEMA_VERSION,
        market_id: args.market_id,
        mint_a: ctx.accounts.mint_a.key(),
        mint_b: ctx.accounts.mint_b.key(),
        token_program: ctx.accounts.token_program.key(),
        admin: ctx.accounts.admin.key(),
        keeper: args.keeper,
        revenue_authority: args.revenue_authority,
        pauser: args.pauser,
        params: args.params,
        // Off until changed through the fee timelock (ADR-0005).
        dynamic_fees: DynamicFeeParams::disabled(args.params.fee_rate_ppm),
        status: MarketStatus::Initializing,
        bump: ctx.bumps.market,
        readiness: args.readiness,
        pending_fees: PendingFees::default(),
        pause_flags: 0,
        rebalancers: [
            args.keeper,
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
        ],
        pending_rebalancers: PendingRebalancers::default(),
        reserved: [0; 14],
    });
    ctx.accounts.registry.set_inner(ShardRegistry {
        market: market_key,
        prepared_mask: 0,
        active_mask: 0,
        registry_version: 0,
        bump: ctx.bumps.registry,
    });

    emit!(MarketInitialized {
        market: market_key,
        mint_a: ctx.accounts.mint_a.key(),
        mint_b: ctx.accounts.mint_b.key(),
        market_id: args.market_id,
        params: args.params,
    });
    Ok(())
}
