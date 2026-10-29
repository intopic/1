//! `initialize_market` — anyone creates `MarketConfig` + empty `ShardRegistry`
//! from a profile the protocol allows (D-05). The creator only pays and picks
//! the profile: spacing, fees and the starting dynamic fee come from the
//! profile, and every role is the protocol's (`ProtocolConfig::roles`).
//!
//! Accounts (R/W/signer):
//! - `creator`        W, signer — payer; recorded as `MarketConfig::creator`, holds no role
//! - `protocol`       R — `["protocol"]`
//! - `mint_a/mint_b`  R — classic SPL Token mints, canonical order, distinct.
//!   A mint with a freeze authority is refused unless it is on the protocol's
//!   allowlist (D-07); a live mint authority only sets a `RISK_*` flag.
//! - `market`         W, init — `["market", schema_u16_le, mint_a, mint_b, market_id_u64_le]`
//! - `registry`       W, init — `["registry", market]`
//! - `token_program`  R — pinned to SPL Token; Token-2022 mints fail the owner check
//! - `system_program` R
//!
//! Post: status `Initializing`, both masks zero, version 0. No funds move.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};

use crate::constants::{
    MARKET_SEED, PROTOCOL_SEED, REGISTRY_SEED, RISK_MINT_AUTHORITY_A, RISK_MINT_AUTHORITY_B,
    SCHEMA_VERSION,
};
use crate::errors::AmmError;
use crate::events::MarketInitialized;
use crate::state::{
    check_mint_order, MarketConfig, MarketStatus, PendingFees, PendingRebalancers, ProtocolConfig,
    ReadinessParams, ShardRegistry,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct InitializeMarketArgs {
    pub market_id: u64,
    /// Index into `ProtocolSettings::profiles`; must be enabled.
    pub profile: u8,
    pub readiness: ReadinessParams,
}

#[derive(Accounts)]
#[instruction(args: InitializeMarketArgs)]
pub struct InitializeMarket<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    pub mint_a: Account<'info, Mint>,
    pub mint_b: Account<'info, Mint>,
    #[account(
        init,
        payer = creator,
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
        payer = creator,
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
    let (mint_a, mint_b) = (&ctx.accounts.mint_a, &ctx.accounts.mint_b);
    check_mint_order(&mint_a.key(), &mint_b.key())?;
    let protocol = &ctx.accounts.protocol;
    let settings = &protocol.settings;
    let profile = settings.profile(args.profile)?;
    let params = profile.params();
    params.validate()?;
    profile.dynamic.validate(params.fee_rate_ppm)?;
    args.readiness.validate()?;
    for mint in [mint_a, mint_b] {
        require!(
            settings.freeze_allowed(&mint.key(), mint.freeze_authority.as_ref().into()),
            AmmError::FreezeAuthorityNotAllowed
        );
    }
    let mut risk_flags = 0;
    if mint_a.mint_authority.is_some() {
        risk_flags |= RISK_MINT_AUTHORITY_A;
    }
    if mint_b.mint_authority.is_some() {
        risk_flags |= RISK_MINT_AUTHORITY_B;
    }
    let roles = protocol.roles;
    let roles_version = protocol.roles_version;

    let market_key = ctx.accounts.market.key();
    ctx.accounts.market.set_inner(MarketConfig {
        schema_version: SCHEMA_VERSION,
        market_id: args.market_id,
        mint_a: ctx.accounts.mint_a.key(),
        mint_b: ctx.accounts.mint_b.key(),
        token_program: ctx.accounts.token_program.key(),
        admin: roles.admin,
        keeper: roles.keeper,
        revenue_authority: roles.revenue_authority,
        pauser: roles.pauser,
        params,
        dynamic_fees: profile.dynamic,
        status: MarketStatus::Initializing,
        bump: ctx.bumps.market,
        readiness: args.readiness,
        pending_fees: PendingFees::default(),
        pause_flags: 0,
        rebalancers: [
            roles.keeper,
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
        ],
        pending_rebalancers: PendingRebalancers::default(),
        config_version: 0,
        creator: ctx.accounts.creator.key(),
        roles_version,
        risk_flags,
        reserved: [0; 10],
    });
    ctx.accounts.registry.set_inner(ShardRegistry {
        market: market_key,
        prepared_mask: 0,
        active_mask: 0,
        registry_version: 0,
        bump: ctx.bumps.registry,
        swap_paused_mask: 0,
    });

    emit!(MarketInitialized {
        market: market_key,
        mint_a: ctx.accounts.mint_a.key(),
        mint_b: ctx.accounts.mint_b.key(),
        market_id: args.market_id,
        params,
        profile: args.profile,
        creator: ctx.accounts.creator.key(),
        risk_flags,
    });
    Ok(())
}
