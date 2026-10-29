//! LP instructions. Each acts on one owner's `Position` in one shard.
//!
//! - `open_position(lower, upper)` creates an empty position.
//! - `increase_liquidity(liquidity, max_a, max_b)` adds liquidity; blocked
//!   while `Paused` or `Cancelled`.
//! - `decrease_liquidity(liquidity, min_a, min_b)` removes liquidity and pays
//!   the principal out. Never blocked by market or shard status (ADR-0002).
//! - `collect_fees()` credits and pays out earned LP fees. Never blocked.
//! - `close_position()` closes an empty position and refunds its rent.
//!
//! Liquidity changes pass the tick arrays holding `lower` and `upper` as 1
//! or 2 writable remaining accounts. Only the position owner can sign.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{POSITION_SEED, VAULT_AUTH_SEED};
use crate::engine::{check_tick_range, modify_position, Pool, PositionState};
use crate::errors::AmmError;
use crate::events::{FeesCollected, LiquidityChanged};
use crate::state::{MarketConfig, Position, ShardState};

use super::token_util::{borrow_all, load_tick_arrays, transfer_in, transfer_out};

#[derive(Accounts)]
#[instruction(tick_lower: i32, tick_upper: i32)]
pub struct OpenPosition<'info> {
    pub owner: Signer<'info>,
    #[account(mut)]
    pub payer: Signer<'info>,
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(has_one = market)]
    pub shard: Account<'info, ShardState>,
    #[account(
        init,
        payer = payer,
        space = 8 + Position::INIT_SPACE,
        seeds = [
            POSITION_SEED,
            shard.key().as_ref(),
            owner.key().as_ref(),
            &tick_lower.to_le_bytes(),
            &tick_upper.to_le_bytes(),
        ],
        bump,
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open_position(
    ctx: Context<OpenPosition>,
    tick_lower: i32,
    tick_upper: i32,
) -> Result<()> {
    let market = &ctx.accounts.market;
    market.check_deposits()?;
    ctx.accounts.shard.check_deposits()?;
    check_tick_range(tick_lower, tick_upper, market.params.tick_spacing)?;
    ctx.accounts.position.set_inner(Position {
        shard: ctx.accounts.shard.key(),
        owner: ctx.accounts.owner.key(),
        tick_lower,
        tick_upper,
        liquidity: 0,
        fee_growth_inside_last_a: [0; 4],
        fee_growth_inside_last_b: [0; 4],
        fees_credited_a: 0,
        fees_credited_b: 0,
        tokens_owed_a: 0,
        tokens_owed_b: 0,
        bump: ctx.bumps.position,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ModifyLiquidity<'info> {
    pub owner: Signer<'info>,
    #[account(has_one = token_program)]
    pub market: Box<Account<'info, MarketConfig>>,
    #[account(mut, has_one = market, has_one = vault_a, has_one = vault_b)]
    pub shard: Box<Account<'info, ShardState>>,
    #[account(mut, has_one = shard, has_one = owner)]
    pub position: Account<'info, Position>,
    #[account(mut)]
    pub vault_a: Box<Account<'info, TokenAccount>>,
    #[account(mut)]
    pub vault_b: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = market.mint_a)]
    pub owner_token_a: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = market.mint_b)]
    pub owner_token_b: Box<Account<'info, TokenAccount>>,
    /// CHECK: PDA signer for the shard vaults; address fixed by seeds.
    #[account(seeds = [VAULT_AUTH_SEED, shard.key().as_ref()], bump = shard.vault_authority_bump)]
    pub vault_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

/// Runs the engine and writes shard, position and tick arrays back.
fn apply<'info>(
    ctx: &mut Context<'info, ModifyLiquidity<'info>>,
    delta: i128,
) -> Result<(u64, u64)> {
    let spacing = ctx.accounts.market.params.tick_spacing;
    let shard_key = ctx.accounts.shard.key();
    let loaders = load_tick_arrays(ctx.remaining_accounts, &shard_key, 2)?;
    let mut refs = borrow_all(&loaders)?;
    let mut arrays: Vec<&mut _> = refs.iter_mut().map(|r| &mut **r).collect();
    let mut pool = Pool::load(&ctx.accounts.shard)?;
    let mut pos = PositionState::load(&ctx.accounts.position);
    let amounts = modify_position(&mut pool, &mut arrays, spacing, &mut pos, delta)?;
    pool.store(&mut ctx.accounts.shard);
    pos.store(&mut ctx.accounts.position);
    Ok(amounts)
}

pub fn handle_increase_liquidity<'info>(
    mut ctx: Context<'info, ModifyLiquidity<'info>>,
    liquidity: u128,
    amount_a_max: u64,
    amount_b_max: u64,
) -> Result<()> {
    ctx.accounts.market.check_deposits()?;
    ctx.accounts.shard.check_deposits()?;
    require!(liquidity > 0, AmmError::ZeroAmount);
    let delta = i128::try_from(liquidity).map_err(|_| AmmError::MathOverflow)?;
    let (a, b) = apply(&mut ctx, delta)?;
    // Any change of liquidity can change what a probe trade does (more
    // liquidity can also make a probe output round to zero), so the depth
    // mark is cleared on increases too (re-audit P2-5).
    ctx.accounts.shard.depth_verified = false;
    require!(
        a <= amount_a_max && b <= amount_b_max,
        AmmError::SlippageExceeded
    );
    let acc = &ctx.accounts;
    transfer_in(
        &acc.token_program,
        &acc.owner_token_a,
        &acc.vault_a,
        &acc.owner,
        a,
    )?;
    transfer_in(
        &acc.token_program,
        &acc.owner_token_b,
        &acc.vault_b,
        &acc.owner,
        b,
    )?;
    emit!(LiquidityChanged {
        shard: acc.shard.key(),
        position: acc.position.key(),
        owner: acc.owner.key(),
        liquidity_delta: delta,
        amount_a: a,
        amount_b: b,
    });
    Ok(())
}

pub fn handle_decrease_liquidity<'info>(
    mut ctx: Context<'info, ModifyLiquidity<'info>>,
    liquidity: u128,
    amount_a_min: u64,
    amount_b_min: u64,
) -> Result<()> {
    require!(liquidity > 0, AmmError::ZeroAmount);
    let delta = i128::try_from(liquidity)
        .map_err(|_| AmmError::MathOverflow)?
        .checked_neg()
        .ok_or(AmmError::MathOverflow)?;
    let (a, b) = apply(&mut ctx, delta)?;
    // Less liquidity may mean less depth: `verify_depth` must run again
    // before a prepared shard can open or activate (also on increases).
    ctx.accounts.shard.depth_verified = false;
    require!(
        a >= amount_a_min && b >= amount_b_min,
        AmmError::SlippageExceeded
    );
    let acc = &ctx.accounts;
    let (shard, bump) = (acc.shard.key(), acc.shard.vault_authority_bump);
    transfer_out(
        &acc.token_program,
        &acc.vault_a,
        &acc.owner_token_a,
        &acc.vault_authority,
        &shard,
        bump,
        a,
    )?;
    transfer_out(
        &acc.token_program,
        &acc.vault_b,
        &acc.owner_token_b,
        &acc.vault_authority,
        &shard,
        bump,
        b,
    )?;
    emit!(LiquidityChanged {
        shard,
        position: acc.position.key(),
        owner: acc.owner.key(),
        liquidity_delta: delta,
        amount_a: a,
        amount_b: b,
    });
    Ok(())
}

pub fn handle_collect_fees<'info>(mut ctx: Context<'info, ModifyLiquidity<'info>>) -> Result<()> {
    apply(&mut ctx, 0)?;
    let acc = &mut ctx.accounts;
    let (a, b) = (acc.position.tokens_owed_a, acc.position.tokens_owed_b);
    acc.position.tokens_owed_a = 0;
    acc.position.tokens_owed_b = 0;
    let (shard, bump) = (acc.shard.key(), acc.shard.vault_authority_bump);
    transfer_out(
        &acc.token_program,
        &acc.vault_a,
        &acc.owner_token_a,
        &acc.vault_authority,
        &shard,
        bump,
        a,
    )?;
    transfer_out(
        &acc.token_program,
        &acc.vault_b,
        &acc.owner_token_b,
        &acc.vault_authority,
        &shard,
        bump,
        b,
    )?;
    emit!(FeesCollected {
        shard,
        position: acc.position.key(),
        owner: acc.owner.key(),
        amount_a: a,
        amount_b: b
    });
    Ok(())
}

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(mut, has_one = owner, close = owner)]
    pub position: Account<'info, Position>,
}

pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    let p = &ctx.accounts.position;
    require!(
        p.liquidity == 0 && p.tokens_owed_a == 0 && p.tokens_owed_b == 0,
        AmmError::PositionNotEmpty
    );
    Ok(())
}
