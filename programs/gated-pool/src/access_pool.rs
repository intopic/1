//! Consumer example for the admission-once API. Legacy pools retain per-action checks.
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use solana_zk_verifier::{access, AccessGrant, AccessPolicy, VerifierState};

use crate::PoolError;

#[account]
#[derive(InitSpace)]
pub struct AccessPool {
    pub authority: Pubkey,
    /// Pinned by the business at creation; depositors cannot substitute a weak policy.
    pub policy: Pubkey,
    pub pool_id: u64,
    pub total_deposits: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct AccessPosition {
    pub owner: Pubkey,
    pub amount: u64,
    pub bump: u8,
}

pub fn create_access_pool_handler(ctx: Context<CreateAccessPool>, pool_id: u64) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    pool.authority = ctx.accounts.authority.key();
    pool.policy = ctx.accounts.policy.key();
    pool.pool_id = pool_id;
    pool.bump = ctx.bumps.pool;
    Ok(())
}

pub fn deposit_access_handler(ctx: Context<DepositAccess>, amount: u64) -> Result<()> {
    require!(amount > 0, PoolError::ZeroAmount);
    access::require_access_grant(
        &ctx.accounts.policy,
        &ctx.accounts.grant,
        &ctx.accounts.verifier_state,
        &ctx.accounts.pool.policy,
        &ctx.accounts.user.key(),
    )?;
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.user.to_account_info(),
                to: ctx.accounts.pool.to_account_info(),
            },
        ),
        amount,
    )?;
    let position = &mut ctx.accounts.position;
    position.owner = ctx.accounts.user.key();
    position.amount = position
        .amount
        .checked_add(amount)
        .ok_or(PoolError::Overflow)?;
    position.bump = ctx.bumps.position;
    ctx.accounts.pool.total_deposits = ctx
        .accounts
        .pool
        .total_deposits
        .checked_add(amount)
        .ok_or(PoolError::Overflow)?;
    Ok(())
}

pub fn withdraw_access_handler(ctx: Context<WithdrawAccess>, amount: u64) -> Result<()> {
    let position = &mut ctx.accounts.position;
    require!(
        amount > 0 && amount <= position.amount,
        PoolError::InsufficientFunds
    );
    position.amount -= amount;
    ctx.accounts.pool.total_deposits -= amount;
    **ctx
        .accounts
        .pool
        .to_account_info()
        .try_borrow_mut_lamports()? -= amount;
    **ctx
        .accounts
        .user
        .to_account_info()
        .try_borrow_mut_lamports()? += amount;
    Ok(())
}

#[derive(Accounts)]
#[instruction(pool_id: u64)]
pub struct CreateAccessPool<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub policy: Account<'info, AccessPolicy>,
    #[account(init, payer = authority, space = 8 + AccessPool::INIT_SPACE,
        seeds = [b"access_pool", authority.key().as_ref(), &pool_id.to_le_bytes()], bump)]
    pub pool: Account<'info, AccessPool>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositAccess<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [b"access_pool", pool.authority.as_ref(), &pool.pool_id.to_le_bytes()], bump = pool.bump)]
    pub pool: Account<'info, AccessPool>,
    #[account(init_if_needed, payer = user, space = 8 + AccessPosition::INIT_SPACE,
        seeds = [b"access_position", pool.key().as_ref(), user.key().as_ref()], bump)]
    pub position: Account<'info, AccessPosition>,
    pub policy: Account<'info, AccessPolicy>,
    pub grant: Account<'info, AccessGrant>,
    pub verifier_state: Box<Account<'info, VerifierState>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawAccess<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [b"access_pool", pool.authority.as_ref(), &pool.pool_id.to_le_bytes()], bump = pool.bump)]
    pub pool: Account<'info, AccessPool>,
    #[account(mut, constraint = position.owner == user.key(),
        seeds = [b"access_position", pool.key().as_ref(), user.key().as_ref()], bump = position.bump)]
    pub position: Account<'info, AccessPosition>,
}
