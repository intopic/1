use anchor_lang::prelude::*;

/// Numri i hyrjeve publike të qarkut `circuits/main.circom`
/// (commitment, attested_at, claim_key_id, bank_key_hash, min_threshold, pubkey_hi, pubkey_lo).
pub const NUM_PUBLIC_INPUTS: usize = 7;
pub const MAX_IC: usize = NUM_PUBLIC_INPUTS + 1;
pub const MAX_BANK_KEYS: usize = 16;

pub mod layout {
    pub const COMMITMENT: usize = 0;
    pub const ATTESTED_AT: usize = 1;
    pub const CLAIM_KEY_ID: usize = 2;
    pub const BANK_KEY_HASH: usize = 3;
    pub const MIN_THRESHOLD: usize = 4;
    pub const USER_PUBKEY_HI: usize = 5;
    pub const USER_PUBKEY_LO: usize = 6;
}

/// Çelësi verifikues Groth16 në kodimin e syscall-eve alt_bn128 (big-endian,
/// G2 me pjesën imagjinare së pari).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace, Debug, PartialEq, Eq)]
pub struct VerifyingKey {
    pub alpha_g1: [u8; 64],
    pub beta_g2: [u8; 128],
    pub gamma_g2: [u8; 128],
    pub delta_g2: [u8; 128],
    #[max_len(MAX_IC)]
    pub ic: Vec<[u8; 64]>,
}

#[account]
#[derive(InitSpace)]
pub struct VerifierState {
    pub authority: Pubkey,
    pub verifying_key: VerifyingKey,
    /// Hash-et Poseidon të modulëve RSA të bankave/noterëve të autorizuar.
    #[max_len(MAX_BANK_KEYS)]
    pub bank_key_whitelist: Vec<[u8; 32]>,
    /// 0 = pa kufi moshe për atestimin.
    pub max_attestation_age_secs: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct AttestationRecord {
    pub user: Pubkey,
    pub threshold_passed: bool,
    pub min_threshold: u64,
    /// Poseidon(value, salt) – vlera mbetet private.
    pub commitment: [u8; 32],
    pub bank_key_hash: [u8; 32],
    pub claim_key_id: u64,
    pub attested_at: i64,
    pub timestamp: i64,
    pub bump: u8,
}

#[event]
pub struct AttestationVerified {
    pub user: Pubkey,
    pub min_threshold: u64,
    pub commitment: [u8; 32],
    pub bank_key_hash: [u8; 32],
    pub attested_at: i64,
}
