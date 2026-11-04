//! Regjistrimi i çelësit verifikues (VK) dhe administrimi i listës së bardhë.

use anchor_lang::prelude::*;

use crate::error::ZkVerifierError;
use crate::program::SolanaZkVerifier;
use crate::state::{VerifierState, VerifyingKey, MAX_BANK_KEYS, MAX_CLAIM_KEYS, MAX_IC};

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Vetëm autoriteti i përditësimit (upgrade authority) mund ta inicializojë:
    /// përndryshe kushdo që e thërret i pari pas deploy-it merr kontrollin e VK-së
    /// dhe të whitelist-ës.
    #[account(constraint = program.programdata_address()? == Some(program_data.key()) @ ZkVerifierError::Unauthorized)]
    pub program: Program<'info, SolanaZkVerifier>,
    #[account(constraint = program_data.upgrade_authority_address == Some(authority.key()) @ ZkVerifierError::Unauthorized)]
    pub program_data: Account<'info, ProgramData>,
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
    app_id: [u8; 32],
    allowed_claim_keys: Vec<u64>,
    max_attestation_age_secs: i64,
) -> Result<()> {
    require!(
        !allowed_claim_keys.is_empty() && allowed_claim_keys.len() <= MAX_CLAIM_KEYS,
        ZkVerifierError::ClaimKeyNotAllowed
    );
    require!(
        !verifying_key.ic.is_empty() && verifying_key.ic.len() <= MAX_IC,
        ZkVerifierError::VerifyingKeyTooLarge
    );
    let state = &mut ctx.accounts.verifier_state;
    state.verifying_key = verifying_key;
    state.authority = ctx.accounts.authority.key();
    state.bank_key_whitelist = Vec::new();
    require!(
        crate::instructions::lt_be(&app_id, &crate::instructions::BN254_FR_MODULUS),
        ZkVerifierError::PublicInputOutOfField
    );
    state.app_id = app_id;
    state.allowed_claim_keys = allowed_claim_keys;
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

/// Shton pika IC në VK. Me 10 hyrje publike VK-ja (≈1.2 KB) nuk nxë në një
/// transaksion (1232 B), ndaj `initialize`/`set_verifying_key` marrin pjesën e
/// parë dhe pjesa tjetër shtohet këtu. Derisa VK të jetë e plotë, verifikimi
/// refuzohet me `PublicInputsLength`.
pub fn extend_vk_ic_handler(ctx: Context<AdminUpdate>, points: Vec<[u8; 64]>) -> Result<()> {
    let ic = &mut ctx.accounts.verifier_state.verifying_key.ic;
    require!(
        ic.len() + points.len() <= MAX_IC,
        ZkVerifierError::VerifyingKeyTooLarge
    );
    ic.extend(points);
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
