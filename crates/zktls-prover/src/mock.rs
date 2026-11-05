//! Noter i simuluar për teste dhe demo: maskon transkriptin, ndërton
//! [`AttestationMessage`] dhe e nënshkruan me RSA-2048 PKCS#1 v1.5 / SHA-256.
//!
//! VETËM PËR TESTIM – çelësi i noterit lexohet nga një skedar PEM lokal.

use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Sign, RsaPrivateKey};
use sha2::{Digest, Sha256};
use triad_core::normalizer::{claim_key_id, AlgebraicNormalizer};

use crate::attestation::{account_tag, AttestationMessage};
use crate::tls_parser::{NotarizedSession, NotaryKey, SignedAttestation};
use crate::ProverError;

/// Përgjigje HTTP shembull nga një "bankë" fiktive.
pub fn sample_bank_response(balance: u64) -> String {
    let body = format!(
        "{{\"name\": \"Arta Hoxha\", \"iban\": \"AL35202111090000000001234567\", \"balance\": {balance}, \"currency\": \"EUR\", \"as_of\": \"2026-11-04\"}}"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: session=7f3a9c1e; Secure; HttpOnly\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// Përgjigje HTTP shembull nga një "regjistër civil" fiktiv (data e lindjes).
pub fn sample_identity_response(date_of_birth: &str) -> String {
    let body = format!(
        "{{\"personal_id\": \"J01234567A\", \"name\": \"Arta Hoxha\", \"date_of_birth\": \"{date_of_birth}\", \"status\": \"active\"}}"
    );
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

pub fn load_private_key_pem(pem: &str) -> Result<RsaPrivateKey, ProverError> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|e| ProverError::Key(e.to_string()))
}

pub struct MockNotary {
    pub key: RsaPrivateKey,
    /// Nëse jepet, sesioni përfshin certifikatën në vend të modulit hex.
    pub certificate_pem: Option<String>,
}

impl MockNotary {
    /// Sekreti i `account_tag` (HMAC), i nxjerrë nga çelësi privat RSA: i qëndrueshëm
    /// sa kohë çelësi nuk ndryshon dhe i panjohur për këdo pa çelësin.
    pub fn tag_key(&self) -> [u8; 32] {
        use rsa::traits::PrivateKeyParts;
        let mut h = Sha256::new();
        h.update(b"zktls/tag-key/v1");
        h.update(self.key.d().to_bytes_be());
        h.finalize().into()
    }

    pub fn notarize(
        &self,
        server_name: &str,
        http_response: &str,
        target_key: &str,
        account_id: &str,
        wallet: [u8; 32],
        timestamp: u64,
    ) -> Result<NotarizedSession, ProverError> {
        let claim =
            AlgebraicNormalizer::normalize_and_mask(http_response.as_bytes(), target_key, 0)?;
        let msg = AttestationMessage {
            value: claim.target_value,
            timestamp,
            claim_key_id: claim_key_id(target_key),
            wallet,
            payload_digest: claim.payload_digest,
            account_tag: account_tag(&self.tag_key(), server_name, account_id),
        };
        let bytes = msg.to_bytes();
        let digest = Sha256::digest(bytes);
        let sig = self
            .key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
            .map_err(|e| ProverError::Key(e.to_string()))?;
        let notary = match &self.certificate_pem {
            Some(c) => NotaryKey {
                certificate_pem: Some(c.clone()),
                modulus_hex: None,
                exponent: 65537,
            },
            None => NotaryKey {
                certificate_pem: None,
                modulus_hex: Some(hex::encode(self.key.n().to_bytes_be())),
                exponent: 65537,
            },
        };
        Ok(NotarizedSession {
            version: 1,
            server_name: server_name.into(),
            target_key: target_key.into(),
            http_response: http_response.into(),
            notary,
            attestation: SignedAttestation {
                message_hex: hex::encode(bytes),
                signature_hex: hex::encode(sig),
            },
        })
    }
}
