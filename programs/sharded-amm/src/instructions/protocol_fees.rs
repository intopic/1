//! `collect_protocol_fees` — the revenue authority withdraws one shard's
//! accrued protocol fees. Destinations must be token accounts owned by the
//! revenue authority with the market's mints. LP principal and LP fees are
//! never reachable from here: only the shard's `protocol_fees_a/b` counters
//! are paid and reset.

use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_SEED;
use crate::state::ProtocolConfig;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::VAULT_AUTH_SEED;
use crate::errors::AmmError;
use crate::events::ProtocolFeesCollected;
use crate::state::{MarketConfig, ShardState};

use super::token_util::transfer_out;

#[derive(Accounts)]
pub struct CollectProtocolFees<'info> {
    pub revenue_authority: Signer<'info>,
    #[account(has_one = revenue_authority, has_one = token_program)]
    pub market: Box<Account<'info, MarketConfig>>,
    /// Current protocol roles; the market's copies must match (D-04).
    #[account(
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.roles_version == market.roles_version @ AmmError::StaleRoles,
    )]
    pub protocol: Box<Account<'info, ProtocolConfig>>,
    #[account(mut, has_one = market, has_one = vault_a, has_one = vault_b)]
    pub shard: Box<Account<'info, ShardState>>,
    #[account(mut)]
    pub vault_a: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_b: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = market.mint_a, token::authority = revenue_authority)]
    pub destination_a: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = market.mint_b, token::authority = revenue_authority)]
    pub destination_b: Box<Account<'info, TokenAccount>>,
    /// CHECK: PDA signer for the shard vaults; address fixed by seeds.
    #[account(seeds = [VAULT_AUTH_SEED, shard.key().as_ref()], bump = shard.vault_authority_bump)]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_collect_protocol_fees(ctx: Context<CollectProtocolFees>) -> Result<()> {
    let shard = &mut ctx.accounts.shard;
    let (a, b) = (shard.protocol_fees_a, shard.protocol_fees_b);
    require!(a > 0 || b > 0, AmmError::ZeroAmount);
    shard.protocol_fees_a = 0;
    shard.protocol_fees_b = 0;
    let acc = &ctx.accounts;
    let (key, bump) = (acc.shard.key(), acc.shard.vault_authority_bump);
    transfer_out(
        &acc.token_program,
        &acc.vault_a,
        &acc.destination_a,
        &acc.vault_authority,
        &key,
        bump,
        a,
    )?;
    transfer_out(
        &acc.token_program,
        &acc.vault_b,
        &acc.destination_b,
        &acc.vault_authority,
        &key,
        bump,
        b,
    )?;
    emit!(ProtocolFeesCollected {
        shard: key,
        amount_a: a,
        amount_b: b,
        by: acc.revenue_authority.key()
    });
    Ok(())
}
