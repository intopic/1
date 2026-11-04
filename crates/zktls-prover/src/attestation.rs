//! Mesazhi kanonik i atestimit (128 bajte) që nënshkruhet me RSA-2048.
//!
//! TLS vetë nuk nënshkruan të dhënat e aplikacionit (vetëm handshake-un), ndaj
//! modeli këtu është ai i një **noteri** (p.sh. TLSNotary/MPC-TLS ose një
//! oracle i bankës): noteri sheh përgjigjen HTTPS, e maskon me MED-01 dhe
//! nënshkruan këtë mesazh fiks. Qarku `circuits/main.circom` llogarit SHA-256
//! e këtyre 128 bajteve brenda provës, ndaj vlera e bilancit është e lidhur
//! kriptografikisht me nënshkrimin.
//!
//! | Bajtet    | Fusha            | Përshkrimi                                     |
//! |-----------|------------------|------------------------------------------------|
//! | 0..8      | magic            | `"ZKTLSv02"`                                   |
//! | 8..16     | value            | u64 BE (bilanci 15200, ose data 20010517)      |
//! | 16..24    | timestamp        | u64 BE, sekonda Unix                           |
//! | 24..32    | claim_key_id     | SHA-256(target_key)[0..8]                      |
//! | 32..64    | wallet           | çelësi publik Solana i përdoruesit             |
//! | 64..96    | payload_digest   | SHA-256(përgjigja HTTP e maskuar)              |
//! | 96..128   | account_tag      | `0x00 ‖ SHA-256(id e llogarisë)[1..32]`        |
//!
//! `account_tag` është identifikues deterministik dhe privat i llogarisë burimore
//! (p.sh. IBAN-i ose user-id). Bajti i parë është 0, ndaj vlera nxë në fushën
//! BN254. Qarku e përdor për nullifier-in `Poseidon(account_tag, app_id)`: e njëjta
//! llogari jep të njëjtin nullifier për një aplikacion, pa u zbuluar (anti-Sybil).

use crate::ProverError;

pub const MAGIC: &[u8; 8] = b"ZKTLSv02";
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

/// `account_tag` deterministik nga identifikuesi i llogarisë (IBAN, user-id, …).
pub fn account_tag(account_id: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"zktls/account-tag/v1");
    h.update(account_id.as_bytes());
    let mut t: [u8; 32] = h.finalize().into();
    t[0] = 0;
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let m = AttestationMessage {
            value: 15200,
            timestamp: 1_700_000_000,
            claim_key_id: 0xdead_beef,
            wallet: [7; 32],
            payload_digest: [9; 32],
            account_tag: account_tag("AL35202111090000000001234567"),
        };
        assert_eq!(AttestationMessage::from_bytes(&m.to_bytes()).unwrap(), m);
        let mut bad = m.to_bytes();
        bad[0] = b'X';
        assert!(AttestationMessage::from_bytes(&bad).is_err());
    }
}
