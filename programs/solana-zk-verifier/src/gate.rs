//! # zk-gate: kontrolli i atestimit për programe të tjera (DEX, launchpad, DAO)
//!
//! Një program që do të lejojë vetëm portofole të verifikuara shton në instruksionin
//! e vet llogarinë `AttestationRecord` dhe thërret [`require_attested`]:
//!
//! ```ignore
//! use solana_zk_verifier::{gate::{self, Requirement}, AttestationRecord};
//!
//! #[account(
//!     seeds = [b"attestation", user.key().as_ref()],
//!     bump = attestation.bump,
//!     seeds::program = solana_zk_verifier::ID,
//! )]
//! pub attestation: Account<'info, AttestationRecord>,
//!
//! // në handler:
//! gate::require_attested(&ctx.accounts.attestation, &user, &Requirement::at_least("balance", 5_000))?;
//! ```
//!
//! `Account<AttestationRecord>` garanton që llogaria i përket programit verifikues
//! (pra prova është verifikuar on-chain); `seeds` garanton që është e këtij portofoli.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;

use crate::{AttestationRecord, ZkVerifierError};

/// `claim_key_id = SHA-256(çelësi JSON)[0..8]` si u64 BE (si në qark dhe në CLI).
pub fn claim_key_id(key: &str) -> u64 {
    let h = hash(key.as_bytes()).to_bytes();
    u64::from_be_bytes(h[..8].try_into().unwrap())
}

/// Çfarë kërkon aplikacioni nga atestimi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub claim_key_id: u64,
    /// Prova duhet të ketë vërtetuar `vlera >= min_value` (ose një prag më të lartë).
    pub min_value: u64,
    /// Prova duhet të ketë vërtetuar `vlera <= max_value` (ose një prag më të ulët).
    pub max_value: u64,
    /// 0 = pa kufi; përndryshe sesioni TLS duhet të jetë më i ri se kaq sekonda.
    pub max_age_secs: i64,
}

impl Requirement {
    /// P.sh. `at_least("balance", 5_000)`: bilanc ≥ 5 000.
    pub fn at_least(claim_key: &str, min_value: u64) -> Self {
        Self {
            claim_key_id: claim_key_id(claim_key),
            min_value,
            max_value: u64::MAX,
            max_age_secs: 0,
        }
    }

    /// P.sh. `at_most("date_of_birth", 20071104)`: i lindur më së voni më 2007-11-04 (18+).
    pub fn at_most(claim_key: &str, max_value: u64) -> Self {
        Self {
            claim_key_id: claim_key_id(claim_key),
            min_value: 0,
            max_value,
            max_age_secs: 0,
        }
    }

    pub fn fresh_within(mut self, secs: i64) -> Self {
        self.max_age_secs = secs;
        self
    }
}

/// Kontrolli i pastër (pa `Clock`), për teste.
pub fn check(record: &AttestationRecord, user: &Pubkey, req: &Requirement, now: i64) -> Result<()> {
    require_keys_eq!(record.user, *user, ZkVerifierError::GateWrongUser);
    require!(
        record.threshold_passed,
        ZkVerifierError::GateThresholdNotMet
    );
    require!(
        record.claim_key_id == req.claim_key_id,
        ZkVerifierError::GateClaimMismatch
    );
    require!(
        record.min_value >= req.min_value && record.max_value <= req.max_value,
        ZkVerifierError::GateThresholdNotMet
    );
    if req.max_age_secs > 0 {
        require!(
            now.saturating_sub(record.attested_at) <= req.max_age_secs,
            ZkVerifierError::GateExpired
        );
    }
    Ok(())
}

/// Kontrolli me orën e zinxhirit.
pub fn require_attested(
    record: &AttestationRecord,
    user: &Pubkey,
    req: &Requirement,
) -> Result<()> {
    check(record, user, req, Clock::get()?.unix_timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(user: Pubkey, min: u64, max: u64, key: &str, at: i64) -> AttestationRecord {
        AttestationRecord {
            user,
            threshold_passed: true,
            min_value: min,
            max_value: max,
            nullifier: [0; 32],
            commitment: [0; 32],
            bank_key_hash: [0; 32],
            claim_key_id: claim_key_id(key),
            attested_at: at,
            timestamp: at,
            bump: 255,
        }
    }

    fn code(r: Result<()>) -> String {
        format!("{:?}", r.unwrap_err())
    }

    #[test]
    fn claim_key_id_matches_cli() {
        // SHA-256("balance")[0..8] BE, e njëjta vlerë si në testet TS
        assert_eq!(claim_key_id("balance"), 6292056758205541950);
    }

    #[test]
    fn balance_gate() {
        let u = Pubkey::new_unique();
        let r = rec(u, 10_000, u64::MAX, "balance", 1_000);
        assert!(check(&r, &u, &Requirement::at_least("balance", 5_000), 1_000).is_ok());
        assert!(check(&r, &u, &Requirement::at_least("balance", 10_000), 1_000).is_ok());
        // prova vërtetoi vetëm ≥ 10 000, jo ≥ 20 000
        assert!(code(check(
            &r,
            &u,
            &Requirement::at_least("balance", 20_000),
            1_000
        ))
        .contains("GateThresholdNotMet"));
        // portofol tjetër
        assert!(code(check(
            &r,
            &Pubkey::new_unique(),
            &Requirement::at_least("balance", 1),
            1_000
        ))
        .contains("GateWrongUser"));
        // pretendim tjetër (një provë bilanci nuk vlen si moshë)
        assert!(code(check(
            &r,
            &u,
            &Requirement::at_most("date_of_birth", 20071104),
            1_000
        ))
        .contains("GateClaimMismatch"));
        // freskia
        let req = Requirement::at_least("balance", 1).fresh_within(3_600);
        assert!(check(&r, &u, &req, 4_600).is_ok());
        assert!(code(check(&r, &u, &req, 4_601)).contains("GateExpired"));
    }

    #[test]
    fn age_gate() {
        let u = Pubkey::new_unique();
        // prova: data e lindjes ≤ 20061104
        let r = rec(u, 0, 20061104, "date_of_birth", 0);
        assert!(check(&r, &u, &Requirement::at_most("date_of_birth", 20071104), 0).is_ok());
        assert!(code(check(
            &r,
            &u,
            &Requirement::at_most("date_of_birth", 20051104),
            0
        ))
        .contains("GateThresholdNotMet"));
    }
}
