//! Parsimi i sesionit TLS të noterizuar dhe i certifikatave X.509.
//!
//! Formati i hyrjes (`session.tls.json`) është një transkript i noterizuar:
//! përgjigja HTTP e deshifruar, çelësi publik RSA i noterit/bankës (certifikatë
//! X.509 PEM ose modul hex) dhe atestimi i nënshkruar (shih [`crate::attestation`]).
//!
//! KUFIZIM: ky crate nuk zbaton protokollin MPC-TLS për të marrë transkriptin nga
//! një lidhje e gjallë; ai pret transkriptin e prodhuar nga një noter.

use serde::{Deserialize, Serialize};
use triad_core::rsa::RsaPublicKey;
use triad_core::{AlgebraicNormalizer, CanonicalClaim};

use crate::attestation::AttestationMessage;
use crate::ProverError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotaryKey {
    /// Certifikata X.509 (PEM) me çelës RSA.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_pem: Option<String>,
    /// Alternativë: moduli RSA big-endian në hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulus_hex: Option<String>,
    #[serde(default = "default_e")]
    pub exponent: u64,
}

fn default_e() -> u64 {
    65537
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedAttestation {
    pub message_hex: String,
    pub signature_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotarizedSession {
    pub version: u32,
    pub server_name: String,
    /// Çelësi JSON i pretendimit (p.sh. `"balance"`).
    pub target_key: String,
    /// Përgjigja HTTP e plotë (headers + trup) e deshifruar nga TLS.
    pub http_response: String,
    /// Shkalla dhjetore e vlerës (politika e burimit): 2 = vlera në centë.
    #[serde(default)]
    pub decimals: u8,
    pub notary: NotaryKey,
    pub attestation: SignedAttestation,
}

/// Rezultati pas të gjitha kontrolleve off-chain.
#[derive(Debug, Clone)]
pub struct VerifiedSession {
    pub message: AttestationMessage,
    pub message_bytes: [u8; 128],
    pub signature: Vec<u8>,
    pub notary_key: RsaPublicKey,
    pub claim: CanonicalClaim,
}

impl NotarizedSession {
    pub fn from_json(s: &str) -> Result<Self, ProverError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn notary_public_key(&self) -> Result<RsaPublicKey, ProverError> {
        if let Some(pem) = &self.notary.certificate_pem {
            return rsa_key_from_certificate_pem(pem.as_bytes());
        }
        if let Some(h) = &self.notary.modulus_hex {
            let n = hex::decode(h.trim_start_matches("0x"))?;
            return Ok(RsaPublicKey::from_be_bytes(&n, self.notary.exponent)?);
        }
        Err(ProverError::Key(
            "mungon certificate_pem ose modulus_hex".into(),
        ))
    }

    /// Verifikon nënshkrimin, lidhjen me transkriptin dhe nxjerr pretendimin.
    pub fn verify(&self, threshold: u64) -> Result<VerifiedSession, ProverError> {
        let key = self.notary_public_key()?;
        if key.size_bytes() != 256 {
            return Err(ProverError::Key(format!(
                "qarku mbështet RSA-2048; çelësi ka {} bit",
                key.size_bytes() * 8
            )));
        }
        let message_bytes_vec = hex::decode(&self.attestation.message_hex)?;
        let signature = hex::decode(&self.attestation.signature_hex)?;
        // 1. Nënshkrimi RSA (MOD-06)
        key.verify_pkcs1v15_sha256(&message_bytes_vec, &signature)?;
        let message = AttestationMessage::from_bytes(&message_bytes_vec)?;
        // 2. Normalizimi (MED-01) i transkriptit
        let claim = AlgebraicNormalizer::normalize_and_mask_scaled(
            self.http_response.as_bytes(),
            &self.target_key,
            threshold,
            self.decimals,
        )?;
        // 3. Integriteti: atestimi i referohet saktësisht këtij transkripti
        if claim.payload_digest != message.payload_digest {
            return Err(ProverError::Attestation(
                "payload_digest nuk përputhet me transkriptin".into(),
            ));
        }
        if claim.target_value != message.value {
            return Err(ProverError::Attestation(format!(
                "vlera e atestuar {} != vlera në transkript {}",
                message.value, claim.target_value
            )));
        }
        if claim.claim_key_id != message.claim_key_id {
            return Err(ProverError::Attestation(
                "claim_key_id nuk përputhet".into(),
            ));
        }
        let mut message_bytes = [0u8; 128];
        message_bytes.copy_from_slice(&message_bytes_vec);
        Ok(VerifiedSession {
            message,
            message_bytes,
            signature,
            notary_key: key,
            claim,
        })
    }
}

/// Nxjerr çelësin RSA nga një certifikatë X.509 në PEM.
pub fn rsa_key_from_certificate_pem(pem: &[u8]) -> Result<RsaPublicKey, ProverError> {
    let (_, pem) =
        x509_parser::pem::parse_x509_pem(pem).map_err(|e| ProverError::Key(e.to_string()))?;
    rsa_key_from_certificate_der(&pem.contents)
}

/// Nxjerr çelësin RSA nga një certifikatë X.509 në DER.
pub fn rsa_key_from_certificate_der(der: &[u8]) -> Result<RsaPublicKey, ProverError> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).map_err(|e| ProverError::Key(e.to_string()))?;
    match cert.public_key().parsed() {
        Ok(x509_parser::public_key::PublicKey::RSA(rsa)) => {
            let e = rsa
                .try_exponent()
                .map_err(|e| ProverError::Key(e.to_string()))?;
            Ok(RsaPublicKey::from_be_bytes(rsa.modulus, e)?)
        }
        Ok(_) => Err(ProverError::Key("certifikata nuk ka çelës RSA".into())),
        Err(e) => Err(ProverError::Key(e.to_string())),
    }
}
