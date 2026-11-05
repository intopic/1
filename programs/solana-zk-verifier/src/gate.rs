//! # zk-gate: kontrolli i atestimit për programe të tjera (DEX, launchpad, DAO)
//!
//! Një program që do të lejojë vetëm portofole të verifikuara shton në instruksionin
//! e vet llogarinë `AttestationRecord` dhe thërret [`require_attested`]:
//!
//! ```ignore
//! use solana_zk_verifier::{gate::{self, Requirement}, AttestationRecord};
//!
//! /// Atestimi zkTLS i portofolit; `require_attested` kontrollon PDA-në dhe pronarin.
//! pub attestation: Account<'info, AttestationRecord>,
//!
//! // në handler:
//! gate::require_attested(&ctx.accounts.attestation, &user, &Requirement::at_least("balance", 5_000))?;
//! gate::require_attested(&ctx.accounts.age, &user, &Requirement::min_age(18, now))?;
//! ```
//!
//! `Account<AttestationRecord>` garanton që llogaria i përket programit verifikues
//! (pra prova është verifikuar on-chain); [`require_attested`] që është PDA-ja
//! `["attestation", portofoli, claim_key_id (u64 LE)]` e këtij portofoli dhe këtij lloji
//! pretendimi. Çdo lloj (bilanc, datëlindje, ...) ka atestimin e vet, të pavarur.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hash;

use crate::{AttestationRecord, ZkVerifierError};

/// `claim_key_id("balance")`
pub const BALANCE: u64 = 6292056758205541950;
/// `claim_key_id("date_of_birth")`
pub const DATE_OF_BIRTH: u64 = 10499278090922323829;

/// Data më e hershme e lindjes që pranohet. Prova e moshës duhet të ketë vërtetuar
/// edhe `datëlindja ≥ kjo datë`, ndaj një "datëlindje" 0 nuk kalon si "mbi 18".
pub const EARLIEST_BIRTH_DATE: u64 = 19_000_101;

/// PDA-ja e atestimit të një portofoli për një lloj pretendimi.
pub fn attestation_address(user: &Pubkey, claim_key_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[b"attestation", user.as_ref(), &claim_key_id.to_le_bytes()],
        &crate::ID,
    )
    .0
}

/// Data e kalendarit (YYYYMMDD) e një kohe Unix (UTC), algoritmi "civil_from_days".
pub fn yyyymmdd(unix: i64) -> u64 {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y * 10_000 + m * 100 + d) as u64
}

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

    /// Mosha ≥ `years` sipas orës së zinxhirit: datëlindja ≤ sot − `years` vjet.
    /// Kufiri llogaritet on-chain, kështu që aplikacioni nuk mund ta gabojë.
    pub fn min_age(years: u64, now_unix: i64) -> Self {
        let today = yyyymmdd(now_unix);
        Self {
            claim_key_id: DATE_OF_BIRTH,
            min_value: EARLIEST_BIRTH_DATE,
            max_value: today.saturating_sub(years.saturating_mul(10_000)),
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

/// Kontrolli i plotë: PDA-ja e saktë (portofol + lloj pretendimi) dhe kushti,
/// me orën e zinxhirit.
pub fn require_attested(
    attestation: &Account<AttestationRecord>,
    user: &Pubkey,
    req: &Requirement,
) -> Result<()> {
    let expected = Pubkey::create_program_address(
        &[
            b"attestation",
            user.as_ref(),
            &req.claim_key_id.to_le_bytes(),
            &[attestation.bump],
        ],
        &crate::ID,
    )
    .map_err(|_| error!(ZkVerifierError::GateWrongAccount))?;
    require_keys_eq!(
        attestation.key(),
        expected,
        ZkVerifierError::GateWrongAccount
    );
    check(attestation, user, req, Clock::get()?.unix_timestamp)
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
    fn constants_and_dates() {
        assert_eq!(claim_key_id("balance"), BALANCE);
        assert_eq!(claim_key_id("date_of_birth"), DATE_OF_BIRTH);
        assert_eq!(yyyymmdd(0), 19700101);
        assert_eq!(yyyymmdd(951_782_400), 20000229); // vit i brishtë
        assert_eq!(yyyymmdd(1_793_750_399), 20261103); // 23:59:59 UTC
        assert_eq!(yyyymmdd(1_793_750_400), 20261104);
        assert_eq!(yyyymmdd(-1), 19691231);
        // 18 vjeç sot (2026-11-04): i lindur më ose para 2008-11-04
        assert_eq!(Requirement::min_age(18, 1_793_750_400).max_value, 20081104);
    }

    #[test]
    fn min_age_boundaries() {
        let u = Pubkey::new_unique();
        let now = 1_793_750_400; // 2026-11-04
                                 // prova u bë dje me kufirin 2008-11-03 → sot vlen (≤ 2008-11-04)
        let r = rec(
            u,
            EARLIEST_BIRTH_DATE,
            20081103,
            "date_of_birth",
            now - 86_400,
        );
        assert!(check(&r, &u, &Requirement::min_age(18, now), now).is_ok());
        // kufiri 2008-11-05 (dikush që mbush 18 nesër) → refuzohet
        let r = rec(u, EARLIEST_BIRTH_DATE, 20081105, "date_of_birth", now);
        assert!(code(check(&r, &u, &Requirement::min_age(18, now), now))
            .contains("GateThresholdNotMet"));
        // prova pa kufi të poshtëm (datëlindja mund të ishte 0) → refuzohet
        let r = rec(u, 0, 20081103, "date_of_birth", now);
        assert!(code(check(&r, &u, &Requirement::min_age(18, now), now))
            .contains("GateThresholdNotMet"));
        // e vjetër: freskia vlen edhe për moshën
        let r = rec(
            u,
            EARLIEST_BIRTH_DATE,
            20081103,
            "date_of_birth",
            now - 2 * 3_600,
        );
        assert!(code(check(
            &r,
            &u,
            &Requirement::min_age(18, now).fresh_within(3_600),
            now
        ))
        .contains("GateExpired"));
        // një provë bilanci nuk kalon si moshë
        let r = rec(u, EARLIEST_BIRTH_DATE, 20081103, "balance", now);
        assert!(
            code(check(&r, &u, &Requirement::min_age(18, now), now)).contains("GateClaimMismatch")
        );
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
