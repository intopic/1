use anchor_lang::prelude::*;

/// Numri i hyrjeve publike të qarkut `circuits/main.circom`.
pub const NUM_PUBLIC_INPUTS: usize = 10;
pub const MAX_IC: usize = NUM_PUBLIC_INPUTS + 1;
pub const MAX_BANK_KEYS: usize = 16;
/// Llojet e pretendimeve (`claim_key_id`) që pranon një instalim.
pub const MAX_CLAIM_KEYS: usize = 4;

pub mod layout {
    // daljet e qarkut
    pub const COMMITMENT: usize = 0;
    pub const ATTESTED_AT: usize = 1;
    pub const CLAIM_KEY_ID: usize = 2;
    pub const NULLIFIER: usize = 3;
    // hyrjet publike
    pub const BANK_KEY_HASH: usize = 4;
    pub const MIN_VALUE: usize = 5;
    pub const MAX_VALUE: usize = 6;
    pub const APP_ID: usize = 7;
    pub const USER_PUBKEY_HI: usize = 8;
    pub const USER_PUBKEY_LO: usize = 9;
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
    /// Identifikuesi i aplikacionit (element Fr, BE). Provat duhet ta kenë këtë
    /// `app_id`, ndaj nullifier-ët janë unikë brenda këtij instalimi.
    pub app_id: [u8; 32],
    /// `claim_key_id` të lejuara (`SHA-256(çelësi JSON)[0..8]`, p.sh. vetëm
    /// `date_of_birth` për një aplikacion moshe). Pa këtë, një provë "bilanc ≤
    /// 20081104" do të kalonte si "i lindur para 2008".
    #[max_len(MAX_CLAIM_KEYS)]
    pub allowed_claim_keys: Vec<u64>,
    /// 0 = pa kufi moshe për atestimin.
    pub max_attestation_age_secs: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct AttestationRecord {
    pub user: Pubkey,
    pub threshold_passed: bool,
    pub min_value: u64,
    pub max_value: u64,
    pub nullifier: [u8; 32],
    /// Poseidon(value, salt) – vlera mbetet private.
    pub commitment: [u8; 32],
    pub bank_key_hash: [u8; 32],
    pub claim_key_id: u64,
    pub attested_at: i64,
    pub timestamp: i64,
    pub bump: u8,
}

/// PDA `["nullifier", nullifier]`: një llogari burimore (IBAN, user-id) mund të
/// lidhet vetëm me një portofol brenda një `app_id` (anti-Sybil).
#[account]
#[derive(InitSpace)]
pub struct NullifierRecord {
    pub owner: Pubkey,
    pub first_used_at: i64,
    pub bump: u8,
}

#[event]
pub struct AttestationVerified {
    pub user: Pubkey,
    pub min_value: u64,
    pub max_value: u64,
    pub nullifier: [u8; 32],
    pub commitment: [u8; 32],
    pub bank_key_hash: [u8; 32],
    pub attested_at: i64,
}
