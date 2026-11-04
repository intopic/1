//! # gated-pool
//!
//! Shembulli minimal se si një DEX/protokoll përdor atestimin zkTLS: një pool
//! depozitash SOL që kërkon, sipas konfigurimit të vet, **bilanc ≥ X**, **moshë ≥ N**,
//! të dyja, ose asnjërën. Kushtet janë të pavarura: një pool vetëm-bilanc nuk
//! kërkon moshë dhe anasjelltas. Tërheqja e parave të veta lejohet gjithmonë.
//!
//! I gjithë integrimi është llogaria e atestimit te [`Deposit`] dhe një rresht
//! `gate::require_attested(...)` për çdo kusht.

// Makrot e Anchor 0.31 gjenerojnë thirrje te `AccountInfo::realloc` (e vjetëruar).
#![allow(deprecated)]

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use solana_zk_verifier::gate::{self, Requirement};
use solana_zk_verifier::AttestationRecord;

// ID placeholder: zëvendësojeni me `anchor keys sync` para deploy-it.
declare_id!("GatedPoo11111111111111111111111111111111111");

#[program]
pub mod gated_pool {
    use super::*;

    /// Krijon pool-in me kushtin e hyrjes: bilanc i vërtetuar ≥ `min_balance`,
    /// atestim jo më i vjetër se `max_age_secs` (0 = pa kufi).
    pub fn create_pool(
        ctx: Context<CreatePool>,
        min_balance: u64,
        max_age_secs: i64,
    ) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        pool.authority = ctx.accounts.authority.key();
        pool.min_balance = min_balance;
        pool.max_age_secs = max_age_secs;
        pool.total_deposits = 0;
        pool.bump = ctx.bumps.pool;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, PoolError::ZeroAmount);
        let user = ctx.accounts.user.key();
        let pool = &ctx.accounts.pool;

        // ---- integrimi zkTLS: një rresht për çdo kusht ----
        if pool.min_balance > 0 {
            let att = ctx
                .accounts
                .balance_attestation
                .as_ref()
                .ok_or(PoolError::MissingAttestation)?;
            gate::require_attested(
                att,
                &user,
                &Requirement::at_least("balance", pool.min_balance).fresh_within(pool.max_age_secs),
            )?;
        }
        if pool.min_age > 0 {
            let att = ctx
                .accounts
                .age_attestation
                .as_ref()
                .ok_or(PoolError::MissingAttestation)?;
            let now = Clock::get()?.unix_timestamp;
            gate::require_attested(att, &user, &Requirement::min_age(pool.min_age as u64, now))?;
        }

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
        position.owner = user;
        position.amount = position
            .amount
            .checked_add(amount)
            .ok_or(PoolError::Overflow)?;
        position.bump = ctx.bumps.position;
        let pool = &mut ctx.accounts.pool;
        pool.total_deposits = pool
            .total_deposits
            .checked_add(amount)
            .ok_or(PoolError::Overflow)?;
        msg!("Depozitë {} lamports nga portofol i verifikuar", amount);
        Ok(())
    }

    /// Tërheqja e parave të veta: pa kontroll atestimi (paratë janë të përdoruesit).
    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let position = &mut ctx.accounts.position;
        require!(
            amount > 0 && amount <= position.amount,
            PoolError::InsufficientFunds
        );
        position.amount -= amount;
        let pool = &mut ctx.accounts.pool;
        pool.total_deposits -= amount;
        **pool.to_account_info().try_borrow_mut_lamports()? -= amount;
        **ctx
            .accounts
            .user
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct Pool {
    pub authority: Pubkey,
    pub min_balance: u64,
    pub min_age: u8,
    pub max_age_secs: i64,
    pub total_deposits: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Position {
    pub owner: Pubkey,
    pub amount: u64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct CreatePool<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Pool::INIT_SPACE, seeds = [b"pool", authority.key().as_ref()], bump)]
    pub pool: Account<'info, Pool>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [b"pool", pool.authority.as_ref()], bump = pool.bump)]
    pub pool: Account<'info, Pool>,
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + Position::INIT_SPACE,
        seeds = [b"position", pool.key().as_ref(), user.key().as_ref()],
        bump,
    )]
    pub position: Account<'info, Position>,
    /// Atestimi "bilanc" i portofolit (kërkohet vetëm nëse `pool.min_balance > 0`).
    /// `Account<AttestationRecord>` kontrollon që i përket programit verifikues;
    /// `require_attested` që është PDA-ja e këtij portofoli dhe këtij lloji.
    pub balance_attestation: Option<Account<'info, AttestationRecord>>,
    /// Atestimi "datëlindje" (kërkohet vetëm nëse `pool.min_age > 0`).
    pub age_attestation: Option<Account<'info, AttestationRecord>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [b"pool", pool.authority.as_ref()], bump = pool.bump)]
    pub pool: Account<'info, Pool>,
    /// `seeds` me portofolin: vetëm pronari e arrin pozicionin e vet
    #[account(mut, seeds = [b"position", pool.key().as_ref(), user.key().as_ref()], bump = position.bump)]
    pub position: Account<'info, Position>,
}

#[error_code]
pub enum PoolError {
    #[msg("Shuma duhet të jetë > 0")]
    ZeroAmount,
    #[msg("Tejkalim numerik")]
    Overflow,
    #[msg("Ky pool kërkon një atestim që nuk u dha")]
    MissingAttestation,
    #[msg("Fonde të pamjaftueshme në pozicion")]
    InsufficientFunds,
}
