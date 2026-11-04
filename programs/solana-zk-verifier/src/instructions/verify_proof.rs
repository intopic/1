//! Verifikimi Groth16 me syscall-et vendase `alt_bn128` dhe ruajtja e atestatit.
//!
//! Kosto e përafërt në CU (tarifat aktuale të syscall-eve):
//! * `alt_bn128_pairing` me 4 çifte: 36 364 + 3 × 12 121 ≈ 72 700 CU
//! * 7 × `alt_bn128_multiplication` (≈ 3 840) + 7 × `alt_bn128_addition` (≈ 334) ≈ 29 200 CU
//! * logjika e Anchor + deserializimi ≈ 10 000 CU
//!
//! Gjithsej ≈ 110k CU. Shifra "~82k" e specifikimit vlen vetëm për pairing-un +
//! pak hyrje publike; numri real duhet matur me `solana-test-validator`.

use anchor_lang::prelude::*;
use solana_bn254::prelude::{alt_bn128_addition, alt_bn128_multiplication, alt_bn128_pairing};

use crate::error::ZkVerifierError;
use crate::state::{
    layout, AttestationRecord, AttestationVerified, NullifierRecord, VerifierState, VerifyingKey,
    NUM_PUBLIC_INPUTS,
};

/// Moduli i fushës bazë Fq të BN254 (big-endian).
pub const BN254_FQ_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c, 0xfd, 0x47,
];
/// Rendi i grupit (fusha skalare Fr) i BN254 (big-endian).
pub const BN254_FR_MODULUS: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

#[derive(Accounts)]
#[instruction(proof_a: [u8; 64], proof_b: [u8; 128], proof_c: [u8; 64], public_inputs: Vec<[u8; 32]>, nullifier: [u8; 32], claim_key_id: u64)]
pub struct VerifyAndAttest<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        seeds = [b"verifier_state"],
        bump = verifier_state.bump,
    )]
    pub verifier_state: Box<Account<'info, VerifierState>>,
    /// Një atestim për çdo (portofol, lloj pretendimi): prova e moshës nuk mbishkruan
    /// provën e bilancit dhe anasjelltas.
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + AttestationRecord::INIT_SPACE,
        seeds = [b"attestation", user.key().as_ref(), &claim_key_id.to_le_bytes()],
        bump,
    )]
    pub attestation_record: Box<Account<'info, AttestationRecord>>,
    /// Një nullifier = një portofol. `init_if_needed` lejon rifreskimin nga i njëjti
    /// portofol; një portofol tjetër refuzohet në handler.
    #[account(
        init_if_needed,
        payer = user,
        space = 8 + NullifierRecord::INIT_SPACE,
        seeds = [b"nullifier", nullifier.as_ref()],
        bump,
    )]
    pub nullifier_record: Box<Account<'info, NullifierRecord>>,
    pub system_program: Program<'info, System>,
}

pub fn lt_be(a: &[u8; 32], m: &[u8; 32]) -> bool {
    a < m // krahasimi leksikografik i bajteve BE == krahasim numerik
}

/// −P për një pikë G1 të koduar `x ‖ y` (BE): `(x, q − y)`.
pub fn negate_g1(p: &[u8; 64]) -> Result<[u8; 64]> {
    let mut out = *p;
    if p.iter().all(|&b| b == 0) {
        return Ok(out);
    }
    let y: [u8; 32] = p[32..].try_into().unwrap();
    require!(lt_be(&y, &BN254_FQ_MODULUS), ZkVerifierError::InvalidPoint);
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let d = BN254_FQ_MODULUS[i] as i16 - y[i] as i16 - borrow;
        out[32 + i] = d.rem_euclid(256) as u8;
        borrow = (d < 0) as i16;
    }
    Ok(out)
}

/// Verifikon `e(−A, B) · e(α, β) · e(vk_x, γ) · e(C, δ) == 1`.
pub fn verify_groth16_bn254(
    proof_a: &[u8; 64],
    proof_b: &[u8; 128],
    proof_c: &[u8; 64],
    public_inputs: &[[u8; 32]],
    vk: &VerifyingKey,
) -> Result<bool> {
    require!(
        public_inputs.len() + 1 == vk.ic.len(),
        ZkVerifierError::PublicInputsLength
    );

    // vk_x = IC[0] + Σ input_i · IC[i+1]
    let mut vk_x = vk.ic[0];
    for (i, input) in public_inputs.iter().enumerate() {
        require!(
            lt_be(input, &BN254_FR_MODULUS),
            ZkVerifierError::PublicInputOutOfField
        );
        let mut mul_in = [0u8; 96];
        mul_in[..64].copy_from_slice(&vk.ic[i + 1]);
        mul_in[64..].copy_from_slice(input);
        let prod = alt_bn128_multiplication(&mul_in)
            .map_err(|_| error!(ZkVerifierError::Bn254SyscallFailed))?;
        let mut add_in = [0u8; 128];
        add_in[..64].copy_from_slice(&vk_x);
        add_in[64..].copy_from_slice(&prod[..64]);
        let sum =
            alt_bn128_addition(&add_in).map_err(|_| error!(ZkVerifierError::Bn254SyscallFailed))?;
        vk_x.copy_from_slice(&sum[..64]);
    }

    let neg_a = negate_g1(proof_a)?;
    let mut pairing_in = [0u8; 4 * 192];
    let pairs: [(&[u8; 64], &[u8; 128]); 4] = [
        (&neg_a, proof_b),
        (&vk.alpha_g1, &vk.beta_g2),
        (&vk_x, &vk.gamma_g2),
        (proof_c, &vk.delta_g2),
    ];
    for (k, (g1, g2)) in pairs.iter().enumerate() {
        pairing_in[k * 192..k * 192 + 64].copy_from_slice(*g1);
        pairing_in[k * 192 + 64..(k + 1) * 192].copy_from_slice(*g2);
    }
    // Pikat e pavlefshme bëjnë që syscall-i të kthejë gabim → prova refuzohet.
    let res = alt_bn128_pairing(&pairing_in).map_err(|_| error!(ZkVerifierError::InvalidProof))?;
    Ok(res.len() == 32 && res[..31].iter().all(|&b| b == 0) && res[31] == 1)
}

/// Lexon një hyrje publike si u64 (24 bajtët e lartë duhet të jenë zero).
fn input_as_u64(x: &[u8; 32]) -> Option<u64> {
    if x[..24].iter().any(|&b| b != 0) {
        return None;
    }
    Some(u64::from_be_bytes(x[24..].try_into().unwrap()))
}

/// Gjysmat 128-bit të çelësit publik, si në `zktls_prover::witness::wallet_halves`.
pub fn wallet_halves(key: &Pubkey) -> ([u8; 32], [u8; 32]) {
    let k = key.to_bytes();
    let (mut hi, mut lo) = ([0u8; 32], [0u8; 32]);
    hi[16..].copy_from_slice(&k[..16]);
    lo[16..].copy_from_slice(&k[16..]);
    (hi, lo)
}

pub fn verify_and_attest_handler(
    ctx: Context<VerifyAndAttest>,
    proof_a: [u8; 64],
    proof_b: [u8; 128],
    proof_c: [u8; 64],
    public_inputs: Vec<[u8; 32]>,
    nullifier: [u8; 32],
    claim_key_id_arg: u64,
) -> Result<()> {
    require!(
        public_inputs.len() == NUM_PUBLIC_INPUTS,
        ZkVerifierError::PublicInputsLength
    );
    let state = &ctx.accounts.verifier_state;
    let user = ctx.accounts.user.key();

    // Rregulli 4: vetëm çelësat e bankave të autorizuara
    let bank_key_hash = public_inputs[layout::BANK_KEY_HASH];
    require!(
        state.bank_key_whitelist.contains(&bank_key_hash),
        ZkVerifierError::BankKeyNotWhitelisted
    );

    // Prova duhet të jetë e lidhur me nënshkruesin (anti-replay nga palë të treta)
    let (hi, lo) = wallet_halves(&user);
    require!(
        public_inputs[layout::USER_PUBKEY_HI] == hi && public_inputs[layout::USER_PUBKEY_LO] == lo,
        ZkVerifierError::WalletMismatch
    );

    require!(
        public_inputs[layout::APP_ID] == state.app_id,
        ZkVerifierError::AppIdMismatch
    );

    let now = Clock::get()?.unix_timestamp;
    let attested_at = input_as_u64(&public_inputs[layout::ATTESTED_AT])
        .and_then(|t| i64::try_from(t).ok())
        .ok_or(ZkVerifierError::StaleAttestation)?;
    require!(
        attested_at <= now.saturating_add(300),
        ZkVerifierError::StaleAttestation
    );
    if state.max_attestation_age_secs > 0 {
        require!(
            now.saturating_sub(attested_at) <= state.max_attestation_age_secs,
            ZkVerifierError::StaleAttestation
        );
    }
    let min_value = input_as_u64(&public_inputs[layout::MIN_VALUE])
        .ok_or(ZkVerifierError::PublicInputOutOfField)?;
    let max_value = input_as_u64(&public_inputs[layout::MAX_VALUE])
        .ok_or(ZkVerifierError::PublicInputOutOfField)?;
    let claim_key_id = input_as_u64(&public_inputs[layout::CLAIM_KEY_ID])
        .ok_or(ZkVerifierError::PublicInputOutOfField)?;
    require!(
        state.allowed_claim_keys.contains(&claim_key_id),
        ZkVerifierError::ClaimKeyNotAllowed
    );
    // argumenti përdoret vetëm për PDA-në; duhet të jetë ai i provës
    require!(
        claim_key_id == claim_key_id_arg,
        ZkVerifierError::ClaimKeyArgMismatch
    );

    let is_valid = verify_groth16_bn254(
        &proof_a,
        &proof_b,
        &proof_c,
        &public_inputs,
        &state.verifying_key,
    )?;
    require!(is_valid, ZkVerifierError::InvalidProof);

    // Anti-Sybil: nullifier-i lidhet me portofolin e parë që e përdor
    // `nullifier` jepet edhe si argument që PDA-ja të nxirret nga IDL; duhet të jetë i njëjti
    require!(
        public_inputs[layout::NULLIFIER] == nullifier,
        ZkVerifierError::NullifierMismatch
    );
    let nrec = &mut ctx.accounts.nullifier_record;
    if nrec.owner == Pubkey::default() {
        nrec.owner = user;
        nrec.first_used_at = now;
        nrec.bump = ctx.bumps.nullifier_record;
    } else {
        require_keys_eq!(nrec.owner, user, ZkVerifierError::NullifierAlreadyUsed);
    }

    let record = &mut ctx.accounts.attestation_record;
    record.user = user;
    record.threshold_passed = true;
    record.min_value = min_value;
    record.max_value = max_value;
    record.nullifier = nullifier;
    record.commitment = public_inputs[layout::COMMITMENT];
    record.bank_key_hash = bank_key_hash;
    record.claim_key_id = claim_key_id;
    record.attested_at = attested_at;
    record.timestamp = now;
    record.bump = ctx.bumps.attestation_record;

    emit!(AttestationVerified {
        user,
        min_value,
        max_value,
        nullifier,
        commitment: record.commitment,
        bank_key_hash,
        attested_at,
    });
    msg!(
        "Prova zkTLS u verifikua: {} <= vlera <= {}",
        min_value,
        max_value
    );
    Ok(())
}
