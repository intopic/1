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
//! | 0..8      | magic            | `"ZKTLSv01"`                                   |
//! | 8..16     | value            | u64 BE (p.sh. bilanci 15200)                   |
//! | 16..24    | timestamp        | u64 BE, sekonda Unix                           |
//! | 24..32    | claim_key_id     | SHA-256(target_key)[0..8]                      |
//! | 32..64    | wallet           | çelësi publik Solana i përdoruesit             |
//! | 64..96    | payload_digest   | SHA-256(përgjigja HTTP e maskuar)              |
//! | 96..128   | nonce            | rastësi e noterit                              |

use crate::ProverError;

pub const MAGIC: &[u8; 8] = b"ZKTLSv01";
pub const LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttestationMessage {
    pub value: u64,
    pub timestamp: u64,
    pub claim_key_id: u64,
    pub wallet: [u8; 32],
    pub payload_digest: [u8; 32],
    pub nonce: [u8; 32],
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
        b[96..128].copy_from_slice(&self.nonce);
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
        let u = |r: core::ops::Range<usize>| u64::from_be_bytes(b[r].try_into().unwrap());
        let a = |r: core::ops::Range<usize>| -> [u8; 32] { b[r].try_into().unwrap() };
        Ok(Self {
            value: u(8..16),
            timestamp: u(16..24),
            claim_key_id: u(24..32),
            wallet: a(32..64),
            payload_digest: a(64..96),
            nonce: a(96..128),
        })
    }
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
            nonce: [1; 32],
        };
        assert_eq!(AttestationMessage::from_bytes(&m.to_bytes()).unwrap(), m);
        let mut bad = m.to_bytes();
        bad[0] = b'X';
        assert!(AttestationMessage::from_bytes(&bad).is_err());
    }
}
