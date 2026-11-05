//! v3 notarized session: raw HTTP response, BabyJubjub public key and EdDSA-Poseidon
//! signature over a fixed 128-byte message. TLSNotary presentation verification
//! happens in bridge/; this parser checks the bridge signature and transcript binding.

use serde::{Deserialize, Serialize};
use triad_core::{AlgebraicNormalizer, CanonicalClaim};

use crate::attestation::AttestationMessage;
use crate::ProverError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotaryKey {
    pub scheme: String,
    pub public_key: crate::eddsa::PublicKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedAttestation {
    pub message_hex: String,
    pub signature: crate::eddsa::Signature,
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
    pub signature: crate::eddsa::Signature,
    pub notary_key: crate::eddsa::PublicKey,
    pub claim: CanonicalClaim,
}

impl NotarizedSession {
    pub fn from_json(s: &str) -> Result<Self, ProverError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn notary_public_key(&self) -> Result<crate::eddsa::PublicKey, ProverError> {
        if self.version != 3 || self.notary.scheme != crate::eddsa::SCHEME {
            return Err(ProverError::Key(
                "expected v3 EdDSA-Poseidon session; legacy RSA requires migration".into(),
            ));
        }
        self.notary.public_key.coordinates()?;
        Ok(self.notary.public_key.clone())
    }

    /// Verifies the signature and the binding to the normalized transcript.
    pub fn verify(&self, threshold: u64) -> Result<VerifiedSession, ProverError> {
        let key = self.notary_public_key()?;
        let raw = hex::decode(&self.attestation.message_hex)?;
        let message = AttestationMessage::from_bytes(&raw)?;
        let message_bytes: [u8; 128] = raw.try_into().expect("validated length");
        let signature = self.attestation.signature.clone();
        key.verify(&message_bytes, &signature)?;
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
        Ok(VerifiedSession {
            message,
            message_bytes,
            signature,
            notary_key: key,
            claim,
        })
    }
}
