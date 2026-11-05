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
    state.policy_epoch = 0;
    state.pending_authority = Pubkey::default();
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
    let state = &mut ctx.accounts.verifier_state;
    state.verifying_key = verifying_key;
    // atestimet e bëra me VK-në e vjetër (p.sh. qarku dev) nuk vlejnë më te gate-i
    state.policy_epoch = state
        .policy_epoch
        .checked_add(1)
        .ok_or(ZkVerifierError::VerifyingKeyTooLarge)?;
    Ok(())
}

/// Revokim masiv: të gjitha atestimet ekzistuese refuzohen nga
/// `gate::require_attested_current` (p.sh. pas komprometimit të një ure).
/// Paratë në pool-e mbeten të tërheqshme; vetëm hyrjet e reja kërkojnë provë të re.
pub fn bump_policy_epoch_handler(ctx: Context<AdminUpdate>) -> Result<()> {
    let state = &mut ctx.accounts.verifier_state;
    state.policy_epoch = state
        .policy_epoch
        .checked_add(1)
        .ok_or(ZkVerifierError::VerifyingKeyTooLarge)?;
    Ok(())
}

/// Afati global i freskisë së sesionit TLS në momentin e verifikimit (0 = pa kufi).
pub fn set_max_attestation_age_handler(ctx: Context<AdminUpdate>, secs: i64) -> Result<()> {
    require!(secs >= 0, ZkVerifierError::StaleAttestation);
    ctx.accounts.verifier_state.max_attestation_age_secs = secs;
    Ok(())
}

/// Hapi 1 i transferimit të autoritetit (p.sh. te një multisig Squads).
/// `Pubkey::default()` anulon një propozim.
pub fn propose_authority_handler(ctx: Context<AdminUpdate>, new_authority: Pubkey) -> Result<()> {
    ctx.accounts.verifier_state.pending_authority = new_authority;
    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAuthority<'info> {
    pub new_authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"verifier_state"],
        bump = verifier_state.bump,
        constraint = verifier_state.pending_authority == new_authority.key() @ ZkVerifierError::Unauthorized,
    )]
    pub verifier_state: Box<Account<'info, VerifierState>>,
}

/// Hapi 2: autoriteti i ri e pranon me nënshkrimin e vet (kështu një adresë e
/// gabuar nuk e bllokon administrimin përgjithmonë).
pub fn accept_authority_handler(ctx: Context<AcceptAuthority>) -> Result<()> {
    let state = &mut ctx.accounts.verifier_state;
    state.authority = ctx.accounts.new_authority.key();
    state.pending_authority = Pubkey::default();
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

/// Ndryshon llojet e pretendimeve që pranon aplikacioni (p.sh. shton `date_of_birth`).
pub fn set_allowed_claim_keys_handler(
    ctx: Context<AdminUpdate>,
    allowed_claim_keys: Vec<u64>,
) -> Result<()> {
    require!(
        allowed_claim_keys.len() <= crate::state::MAX_CLAIM_KEYS,
        ZkVerifierError::WhitelistFull
    );
    ctx.accounts.verifier_state.allowed_claim_keys = allowed_claim_keys;
    Ok(())
}
