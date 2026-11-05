//! Canonical v3 attestation message, signed with BabyJubjub EdDSA-Poseidon.
//! The circuit hashes all 128 bytes as five BE limbs (<=31 bytes each), with a
//! protocol domain. SHA-256 remains only the masked HTTP payload digest.
//!
//! | Bajtet    | Fusha            | Përshkrimi                                     |
//! |-----------|------------------|------------------------------------------------|
//! | 0..8      | magic            | `"ZKTLSv03"`                                   |
//! | 8..16     | value            | u64 BE (bilanci 15200, ose data 20010517)      |
//! | 16..24    | timestamp        | u64 BE, sekonda Unix                           |
//! | 24..32    | claim_key_id     | SHA-256(target_key)[0..8]                      |
//! | 32..64    | wallet           | çelësi publik Solana i përdoruesit             |
//! | 64..96    | payload_digest   | SHA-256(përgjigja HTTP e maskuar)              |
//! | 96..128   | account_tag      | `0x00 ‖ HMAC(çelësi i noterit, burim ‖ id)[1..32]` |
//!
//! `account_tag` është identifikues deterministik dhe privat i llogarisë burimore
//! (p.sh. IBAN-i ose user-id). Bajti i parë është 0, ndaj vlera nxë në fushën
//! BN254. Qarku e përdor për nullifier-in `Poseidon(account_tag, app_id)`: e njëjta
//! llogari jep të njëjtin nullifier për një aplikacion, pa u zbuluar (anti-Sybil).
//!
//! * **I fshehtë (HMAC me çelës):** pa çelësin e noterit nuk mund të provohen ID të
//!   hamendësueshme (user_id 1..100, IBAN të njohura) kundrejt nullifier-it publik.
//!   Vetëm operatori i noterit/urës e ka këtë çelës.
//! * **I ndarë sipas burimit:** domeni i serverit hyn në HMAC, ndaj `user_id=42` te
//!   dy banka të ndryshme jep nullifier-ë të ndryshëm.

use crate::ProverError;

pub const MAGIC: &[u8; 8] = b"ZKTLSv03";
pub const LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttestationMessage {
    pub value: u64,
    pub timestamp: u64,
    pub claim_key_id: u64,
    pub wallet: [u8; 32],
    pub payload_digest: [u8; 32],
    pub account_tag: [u8; 32],
}

impl AttestationMessage {
    pub fn to_bytes(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[0..8].copy_from_slice(MAGIC);
        b[8..16].copy_from_slice(&self.value.to_be_bytes());
        b[16..24].copy_from_slice(&self.timestamp.to_be_bytes());
        b[24..32].copy_from_slice(&self.claim_key_id.to_be_bytes());
        b[32..64].copy_from_slice(&self.wallet);
        b[64..96].copy_from_slice(&self.payload_digest);
        b[96..128].copy_from_slice(&self.account_tag);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, ProverError> {
        if b.len() != LEN {
            return Err(ProverError::Attestation(format!(
                "gjatësia {} != {LEN}",
                b.len()
            )));
        }
        if &b[0..8] != MAGIC {
            return Err(ProverError::Attestation("magic i gabuar".into()));
        }
        if b[96] != 0 {
            return Err(ProverError::Attestation(
                "account_tag[0] duhet të jetë 0".into(),
            ));
        }
        let u = |r: core::ops::Range<usize>| u64::from_be_bytes(b[r].try_into().unwrap());
        let a = |r: core::ops::Range<usize>| -> [u8; 32] { b[r].try_into().unwrap() };
        Ok(Self {
            value: u(8..16),
            timestamp: u(16..24),
            claim_key_id: u(24..32),
            wallet: a(32..64),
            payload_digest: a(64..96),
            account_tag: a(96..128),
        })
    }
}

/// `account_tag` = `0x00 ‖ HMAC-SHA256(tag_key, "zktls/account-tag/v2" ‖ len(server) ‖ server ‖ id)[1..32]`.
///
/// `tag_key` është sekret i noterit (shih `MockNotary::tag_key`); `server_name` është
/// domeni i autentikuar i burimit.
///
/// Hapësirat hiqen nga `account_id` para hash-it: e njëjta llogari e shkruar
/// `AL35 2021 1109 ...` dhe `AL352021 1109...` jep të njëjtin tag (pa këtë, dy formate
/// të të njëjtit IBAN do të jepnin dy nullifier-a, pra dy portofole).
pub fn account_tag(tag_key: &[u8], server_name: &str, account_id: &str) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(tag_key).expect("HMAC pranon çdo gjatësi");
    mac.update(b"zktls/account-tag/v2");
    mac.update(&(server_name.len() as u64).to_be_bytes());
    mac.update(server_name.as_bytes());
    let id: String = account_id.chars().filter(|c| !c.is_whitespace()).collect();
    mac.update(id.as_bytes());
    let mut t: [u8; 32] = mac.finalize().into_bytes().into();
    t[0] = 0;
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_tag_is_keyed_and_per_source() {
        let t = account_tag(b"sekret", "api.banka-a.al", "42");
        assert_eq!(t[0], 0);
        assert_eq!(t, account_tag(b"sekret", "api.banka-a.al", "42"));
        // burim tjetër, e njëjta ID → tag tjetër (pa përplasje mes bankave)
        assert_ne!(t, account_tag(b"sekret", "api.banka-b.al", "42"));
        // pa çelësin nuk rikuperohet: çelës tjetër → tag tjetër
        assert_ne!(t, account_tag(b"tjeter", "api.banka-a.al", "42"));
        // ndarja server/id është e paqartë pa gjatësinë: "a.al"+"b42" ≠ "a.alb"+"42"
        assert_eq!(
            account_tag(b"k", "a.al", "AL35 2021 1109"),
            account_tag(b"k", "a.al", "AL3520211109")
        );
        assert_ne!(
            account_tag(b"k", "a.al", "b42"),
            account_tag(b"k", "a.alb", "42")
        );
    }
    #[test]
    fn roundtrip() {
        let m = AttestationMessage {
            value: 15200,
            timestamp: 1_700_000_000,
            claim_key_id: 0xdead_beef,
            wallet: [7; 32],
            payload_digest: [9; 32],
            account_tag: account_tag(b"k", "api.banka.al", "AL35202111090000000001234567"),
        };
        assert_eq!(AttestationMessage::from_bytes(&m.to_bytes()).unwrap(), m);
        let mut bad = m.to_bytes();
        bad[0] = b'X';
        assert!(AttestationMessage::from_bytes(&bad).is_err());
    }
}
