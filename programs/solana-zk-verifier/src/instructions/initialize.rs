//! Regjistrimi i çelësit verifikues (VK) dhe administrimi i listës së bardhë.

use anchor_lang::prelude::*;

use crate::error::ZkVerifierError;
use crate::state::{VerifierState, VerifyingKey, MAX_BANK_KEYS, MAX_IC};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + VerifierState::INIT_SPACE,
        seeds = [b"verifier_state"],
        bump,
    )]
    pub verifier_state: Box<Account<'info, VerifierState>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminUpdate<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"verifier_state"],
        bump = verifier_state.bump,
        has_one = authority,
    )]
    pub verifier_state: Box<Account<'info, VerifierState>>,
}

pub fn initialize_handler(
    ctx: Context<Initialize>,
    verifying_key: VerifyingKey,
    max_attestation_age_secs: i64,
) -> Result<()> {
    require!(
        !verifying_key.ic.is_empty() && verifying_key.ic.len() <= MAX_IC,
        ZkVerifierError::VerifyingKeyTooLarge
    );
    let state = &mut ctx.accounts.verifier_state;
    state.verifying_key = verifying_key;
    state.authority = ctx.accounts.authority.key();
    state.bank_key_whitelist = Vec::new();
    state.max_attestation_age_secs = max_attestation_age_secs.max(0);
    state.bump = ctx.bumps.verifier_state;
    Ok(())
}

pub fn set_verifying_key_handler(
    ctx: Context<AdminUpdate>,
    verifying_key: VerifyingKey,
) -> Result<()> {
    require!(
        !verifying_key.ic.is_empty() && verifying_key.ic.len() <= MAX_IC,
        ZkVerifierError::VerifyingKeyTooLarge
    );
    ctx.accounts.verifier_state.verifying_key = verifying_key;
    Ok(())
}

pub fn add_bank_key_handler(ctx: Context<AdminUpdate>, bank_key_hash: [u8; 32]) -> Result<()> {
    let list = &mut ctx.accounts.verifier_state.bank_key_whitelist;
    require!(
        !list.contains(&bank_key_hash),
        ZkVerifierError::BankKeyAlreadyWhitelisted
    );
    require!(list.len() < MAX_BANK_KEYS, ZkVerifierError::WhitelistFull);
    list.push(bank_key_hash);
    Ok(())
}

pub fn remove_bank_key_handler(ctx: Context<AdminUpdate>, bank_key_hash: [u8; 32]) -> Result<()> {
    let list = &mut ctx.accounts.verifier_state.bank_key_whitelist;
    let before = list.len();
    list.retain(|k| k != &bank_key_hash);
    require!(list.len() < before, ZkVerifierError::BankKeyNotWhitelisted);
    Ok(())
}
