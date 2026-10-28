//! `prepare_shard` — creates one shard's state, vault authority and two vaults,
//! and records its prepared bit. The shard stays `Prepared` (not routable).
//!
//! Accounts (R/W/signer):
//! - `authority`        signer — keeper, or admin while `Initializing`
//! - `payer`            W, signer — rent
//! - `market`           R — binds mints and token program
//! - `registry`         W — `["registry", market]`; prepared bit + version
//! - `mint_a/mint_b`    R — must equal `market.mint_a/mint_b`
//! - `shard`            W, init — `["shard", market, id_u8, generation_u64_le]`
//! - `vault_authority`  R — `["vault_auth", shard]`, data-less PDA signer
//! - `vault_a/vault_b`  W, init — `["vault", shard, mint]`, token accounts owned by `vault_authority`
//! - `token_program`, `system_program`, `rent` R
//!
//! Invariants: vault A/B are distinct accounts (different mint seeds), each
//! shard has its own vaults and authority, so no vault is shared between
//! shards. `vault_authority` is never given to any role.

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::{REGISTRY_SEED, SHARD_SEED, VAULT_AUTH_SEED, VAULT_SEED};
use crate::errors::AmmError;
use crate::events::ShardPrepared;
use crate::state::{MarketConfig, ShardRegistry, ShardState, ShardStatus};

#[derive(Accounts)]
#[instruction(id: u8, generation: u64)]
pub struct PrepareShard<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(has_one = mint_a, has_one = mint_b, has_one = token_program)]
    pub market: Account<'info, MarketConfig>,
    #[account(
        mut,
        seeds = [REGISTRY_SEED, market.key().as_ref()],
        bump = registry.bump,
        has_one = market,
    )]
    pub registry: Account<'info, ShardRegistry>,
    pub mint_a: Account<'info, Mint>,
    pub mint_b: Account<'info, Mint>,
    #[account(
        init,
        payer = payer,
        space = 8 + ShardState::INIT_SPACE,
        seeds = [SHARD_SEED, market.key().as_ref(), &[id], &generation.to_le_bytes()],
        bump,
    )]
    pub shard: Account<'info, ShardState>,
    /// CHECK: data-less PDA used only as the vaults' token authority. Its
    /// address is fixed by the seeds constraint; it holds no data and is
    /// never read. Signing happens only inside this program's CPIs.
    #[account(seeds = [VAULT_AUTH_SEED, shard.key().as_ref()], bump)]
    pub vault_authority: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        seeds = [VAULT_SEED, shard.key().as_ref(), mint_a.key().as_ref()],
        bump,
        token::mint = mint_a,
        token::authority = vault_authority,
    )]
    pub vault_a: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = payer,
        seeds = [VAULT_SEED, shard.key().as_ref(), mint_b.key().as_ref()],
        bump,
        token::mint = mint_b,
        token::authority = vault_authority,
    )]
    pub vault_b: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_prepare_shard(ctx: Context<PrepareShard>, id: u8, generation: u64) -> Result<()> {
    let market = &ctx.accounts.market;
    require!(
        market.can_prepare(&ctx.accounts.authority.key()),
        AmmError::Unauthorized
    );
    require!(generation == 0, AmmError::UnsupportedGeneration);
    // Range, market status, duplicate and version checks; commits only on success.
    ctx.accounts.registry.record_prepared(market.status, id)?;

    let shard_key = ctx.accounts.shard.key();
    ctx.accounts.shard.set_inner(ShardState {
        market: market.key(),
        id,
        generation,
        status: ShardStatus::Prepared,
        vault_a: ctx.accounts.vault_a.key(),
        vault_b: ctx.accounts.vault_b.key(),
        vault_authority_bump: ctx.bumps.vault_authority,
        sqrt_price_x64: 0,
        tick_current: 0,
        liquidity: 0,
        fee_growth_global_a_x64: 0,
        fee_growth_global_b_x64: 0,
        protocol_fees_a: 0,
        protocol_fees_b: 0,
        bump: ctx.bumps.shard,
        reserved: [0; 64],
    });

    emit!(ShardPrepared {
        market: market.key(),
        shard: shard_key,
        id,
        generation,
        registry_version: ctx.accounts.registry.registry_version,
    });
    Ok(())
}
