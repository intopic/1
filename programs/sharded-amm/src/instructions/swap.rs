//! `swap_exact_in` — swaps a fixed input on one active shard.
//!
//! Accounts (R/W/signer):
//! - `user` signer; `market` R; `registry` R — both read-only
//! - `shard` W (`has_one = market, vault_a, vault_b`); `vault_a`, `vault_b` W
//! - `user_token_in`, `user_token_out` W — mints must match the direction
//! - `vault_authority` R (PDA signer); `token_program`
//! - remaining: 1..=3 writable tick arrays of this shard, starting with the
//!   one containing the current tick, in swap direction
//!
//! The fee is the base fee or, when enabled, the shard's dynamic fee for the
//! current slot (ADR-0005); its state is in `shard`, so nothing shared is
//! written.
//!
//! The whole input is consumed or the transaction fails. Checks after the
//! math and before any transfer: `amount_out >= min_amount_out`,
//! `rounding_dust <= max_rounding_dust`, `now <= deadline`.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{MAX_SWAP_TICK_ARRAYS, REGISTRY_SEED, VAULT_AUTH_SEED};
use crate::engine::{dynamic_fee, swap, Pool, SwapParams};
use crate::errors::AmmError;
use crate::events::SwapExecuted;
use crate::state::{MarketConfig, ShardRegistry, ShardState, ShardStatus};

use super::token_util::{borrow_all, load_tick_arrays, transfer_in, transfer_out};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct SwapArgs {
    pub a_to_b: bool,
    pub amount_in: u64,
    pub min_amount_out: u64,
    /// 0 = no limit beyond the domain.
    pub sqrt_price_limit_x64: u128,
    pub max_rounding_dust: u64,
    /// Unix seconds; the swap fails after this time.
    pub deadline: i64,
}

#[derive(Accounts)]
pub struct SwapExactIn<'info> {
    pub user: Signer<'info>,
    #[account(has_one = token_program)]
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(seeds = [REGISTRY_SEED, market.key().as_ref()], bump = registry.bump, has_one = market)]
    pub registry: Box<Account<'info, ShardRegistry>>,
    #[account(mut, has_one = market, has_one = vault_a, has_one = vault_b)]
    pub shard: Box<Account<'info, ShardState>>,
    #[account(mut)]
    pub vault_a: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_b: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub user_token_in: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub user_token_out: Box<Account<'info, TokenAccount>>,
    /// CHECK: PDA signer for the shard vaults; address fixed by seeds.
    #[account(seeds = [VAULT_AUTH_SEED, shard.key().as_ref()], bump = shard.vault_authority_bump)]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

pub fn handle_swap_exact_in<'info>(
    ctx: Context<'info, SwapExactIn<'info>>,
    args: SwapArgs,
) -> Result<()> {
    require!(
        Clock::get()?.unix_timestamp <= args.deadline,
        AmmError::DeadlineExceeded
    );
    let acc = &ctx.accounts;
    acc.registry
        .check_swappable(acc.market.status, acc.shard.id)?;
    acc.market.check_swaps()?;
    require!(
        acc.shard.status == ShardStatus::Active,
        AmmError::ShardNotActive
    );
    let (mint_in, mint_out) = if args.a_to_b {
        (acc.market.mint_a, acc.market.mint_b)
    } else {
        (acc.market.mint_b, acc.market.mint_a)
    };
    require!(
        acc.user_token_in.mint == mint_in && acc.user_token_out.mint == mint_out,
        AmmError::InvalidTokenAccount
    );

    let shard_key = acc.shard.key();
    let params = acc.market.params;
    let dynamic = acc.market.dynamic_fees;
    let slot = Clock::get()?.slot;
    let loaders = load_tick_arrays(ctx.remaining_accounts, &shard_key, MAX_SWAP_TICK_ARRAYS)?;
    let mut refs = borrow_all(&loaders)?;
    let mut arrays: Vec<&mut _> = refs.iter_mut().map(|r| &mut **r).collect();
    let mut pool = Pool::load(&acc.shard)?;
    let mut fee_state = acc.shard.dynamic_fee;
    let fee_rate_ppm = dynamic_fee::begin(
        &mut fee_state,
        &dynamic,
        params.fee_rate_ppm,
        params.tick_spacing,
        pool.tick,
        slot,
    );
    let out = swap(
        &mut pool,
        &mut arrays,
        SwapParams {
            a_to_b: args.a_to_b,
            amount_in: args.amount_in,
            sqrt_price_limit: args.sqrt_price_limit_x64,
            fee_rate_ppm,
            protocol_share_bps: params.protocol_share_bps,
            tick_spacing: params.tick_spacing,
        },
    )?;
    require!(
        out.amount_out >= args.min_amount_out,
        AmmError::SlippageExceeded
    );
    require!(
        out.rounding_dust <= args.max_rounding_dust,
        AmmError::RoundingDustExceeded
    );
    drop(arrays);
    drop(refs);
    dynamic_fee::finish(
        &mut fee_state,
        &dynamic,
        params.tick_spacing,
        pool.tick,
        slot,
    );
    pool.store(&mut ctx.accounts.shard);
    ctx.accounts.shard.dynamic_fee = fee_state;

    let acc = &ctx.accounts;
    let bump = acc.shard.vault_authority_bump;
    let (vault_in, vault_out) = if args.a_to_b {
        (&acc.vault_a, &acc.vault_b)
    } else {
        (&acc.vault_b, &acc.vault_a)
    };
    transfer_in(
        &acc.token_program,
        &acc.user_token_in,
        vault_in,
        &acc.user,
        out.amount_in,
    )?;
    transfer_out(
        &acc.token_program,
        vault_out,
        &acc.user_token_out,
        &acc.vault_authority,
        &shard_key,
        bump,
        out.amount_out,
    )?;
    emit!(SwapExecuted {
        shard: shard_key,
        user: acc.user.key(),
        a_to_b: args.a_to_b,
        amount_in: out.amount_in,
        amount_out: out.amount_out,
        total_fee: out.total_fee,
        protocol_fee: out.protocol_fee,
        rounding_dust: out.rounding_dust,
        sqrt_price_x64: pool.sqrt_price,
        tick: pool.tick,
        ticks_crossed: out.ticks_crossed,
        fee_rate_ppm,
        liquidity: pool.liquidity,
    });
    Ok(())
}
