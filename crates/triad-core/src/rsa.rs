//! Verifikimi i nënshkrimeve RSASSA-PKCS1-v1_5 me SHA-256 mbi motorin MOD-06.

use alloc::vec::Vec;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::bigint;
use crate::euclid::{EuclidCore, EuclidError};
use crate::montgomery::{MontgomeryEngine, MontgomeryError};

/// Prefiksi DER `DigestInfo` për SHA-256 (RFC 8017 §9.2, shënimi 1).
pub const SHA256_DIGEST_INFO: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsaError {
    InvalidModulus(MontgomeryError),
    InvalidExponent(EuclidError),
    ModulusTooSmall,
    BadSignatureLength,
    SignatureOutOfRange,
    InvalidSignature,
}

impl core::fmt::Display for RsaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RsaError {}

#[derive(Clone, Debug)]
pub struct RsaPublicKey {
    engine: MontgomeryEngine,
    e: u64,
    /// Gjatësia e modulit në bajte (`k` në RFC 8017).
    k: usize,
}

impl RsaPublicKey {
    /// Ndërton çelësin nga moduli big-endian dhe eksponenti publik.
    pub fn from_be_bytes(modulus_be: &[u8], e: u64) -> Result<Self, RsaError> {
        let first = modulus_be
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(modulus_be.len());
        let n = &modulus_be[first..];
        if n.len() < 64 {
            return Err(RsaError::ModulusTooSmall);
        }
        let limbs = bigint::from_be_bytes(n, n.len().div_ceil(8)).expect("gjatësi e llogaritur");
        let engine = MontgomeryEngine::try_new(&limbs).map_err(RsaError::InvalidModulus)?;
        EuclidCore::validate_public_exponent(e, &limbs).map_err(RsaError::InvalidExponent)?;
        Ok(Self {
            engine,
            e,
            k: n.len(),
        })
    }

    pub fn modulus_limbs(&self) -> &[u64] {
        &self.engine.modulus
    }
    pub fn modulus_be_bytes(&self) -> Vec<u8> {
        bigint::to_be_bytes(&self.engine.modulus, self.k)
    }
    pub fn exponent(&self) -> u64 {
        self.e
    }
    pub fn size_bytes(&self) -> usize {
        self.k
    }

    /// `s^e mod n` (operacion publik, kohë e ndryshueshme sipas `e`).
    pub fn raw_public_op(&self, sig_be: &[u8]) -> Result<Vec<u8>, RsaError> {
        if sig_be.len() != self.k {
            return Err(RsaError::BadSignatureLength);
        }
        let n = self.engine.limbs();
        let s = bigint::from_be_bytes(sig_be, n).ok_or(RsaError::SignatureOutOfRange)?;
        if !bool::from(bigint::ct_lt(&s, &self.engine.modulus)) {
            return Err(RsaError::SignatureOutOfRange);
        }
        let mut m = alloc::vec![0u64; n];
        self.engine.exp_vartime(&s, &[self.e], &mut m);
        Ok(bigint::to_be_bytes(&m, self.k))
    }

    /// Kodimi EMSA-PKCS1-v1_5 i një hash-i SHA-256 me gjatësi `k`.
    pub fn emsa_pkcs1_v15_sha256(&self, digest: &[u8; 32]) -> Vec<u8> {
        let mut em = alloc::vec![0xffu8; self.k];
        em[0] = 0x00;
        em[1] = 0x01;
        let t_len = SHA256_DIGEST_INFO.len() + 32;
        em[self.k - t_len - 1] = 0x00;
        em[self.k - t_len..self.k - 32].copy_from_slice(&SHA256_DIGEST_INFO);
        em[self.k - 32..].copy_from_slice(digest);
        em
    }

    pub fn verify_prehashed(&self, digest: &[u8; 32], sig_be: &[u8]) -> Result<(), RsaError> {
        let em = self.raw_public_op(sig_be)?;
        let expected = self.emsa_pkcs1_v15_sha256(digest);
        if bool::from(em.ct_eq(&expected)) {
            Ok(())
        } else {
            Err(RsaError::InvalidSignature)
        }
    }

    pub fn verify_pkcs1v15_sha256(&self, message: &[u8], sig_be: &[u8]) -> Result<(), RsaError> {
        let digest: [u8; 32] = Sha256::digest(message).into();
        self.verify_prehashed(&digest, sig_be)
    }
}
