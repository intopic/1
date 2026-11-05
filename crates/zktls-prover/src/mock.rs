//! Noter i simuluar për teste dhe demo: maskon transkriptin, ndërton
//! [`AttestationMessage`] dhe e nënshkruan me EdDSA-Poseidon.
//!
//! VETËM PËR TESTIM – çelësi i noterit lexohet nga një skedar seed hex lokal.

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

pub struct MockNotary {
    pub key: crate::eddsa::SigningKey,
    /// Sekreti i qëndrueshëm i `account_tag`, i ndarë nga çelësi EdDSA i nënshkrimit.
    /// Me të, rotacioni i çelësit EdDSA nuk i ndryshon nullifier-ët (pra një llogari
    /// nuk mund të regjistrohet sërish me portofol tjetër pas rotacionit).
    /// `None` = nxirret nga çelësi EdDSA (vetëm demo/teste).
    pub tag_secret: Option<[u8; 32]>,
}

/// Çfarë nënshkruan lëshuesi për një përgjigje të verifikuar.
pub struct NotarizeRequest<'a> {
    /// Domeni TLS i burimit (shkruhet në sesion).
    pub server_name: &'a str,
    /// Identiteti i qëndrueshëm i lëshuesit për `account_tag` (p.sh. "banka-x"), që
    /// mbetet i njëjtë kur banka ndërron domen ose ka disa domene. `None` = `server_name`.
    pub issuer: Option<&'a str>,
    pub http_response: &'a str,
    pub target_key: &'a str,
    pub account_id: &'a str,
    pub wallet: [u8; 32],
    pub timestamp: u64,
    /// Shkalla dhjetore e vlerës (0 = njësi të plota, 2 = centë).
    pub decimals: u8,
}

impl MockNotary {
    /// Sekreti i `account_tag` (HMAC): `tag_secret` nëse jepet (prodhim), përndryshe
    /// i nxjerrë nga çelësi privat EdDSA (i qëndrueshëm vetëm sa kohë çelësi nuk ndryshon).
    pub fn tag_key(&self) -> [u8; 32] {
        self.tag_secret.unwrap_or_else(|| self.key.demo_tag_key())
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
        self.notarize_with(&NotarizeRequest {
            server_name,
            issuer: None,
            http_response,
            target_key,
            account_id,
            wallet,
            timestamp,
            decimals: 0,
        })
    }

    pub fn notarize_with(&self, r: &NotarizeRequest) -> Result<NotarizedSession, ProverError> {
        let NotarizeRequest {
            server_name,
            issuer,
            http_response,
            target_key,
            account_id,
            wallet,
            timestamp,
            decimals,
        } = *r;
        // një ID bosh (ose vetëm hapësira) do t'u jepte të gjithëve të njëjtin nullifier
        if account_id.chars().all(char::is_whitespace) {
            return Err(ProverError::Attestation(
                "identifikuesi i llogarisë është bosh".into(),
            ));
        }
        let claim = AlgebraicNormalizer::normalize_and_mask_scaled(
            http_response.as_bytes(),
            target_key,
            0,
            decimals,
        )?;
        // një datëlindje duhet të jetë datë reale (jo 20010230 ose një sentinel numerik)
        if crate::witness::BIRTH_DATE_KEYS.contains(&target_key)
            && (decimals != 0 || !triad_core::is_calendar_date(claim.target_value))
        {
            return Err(ProverError::Attestation(format!(
                "`{target_key}` nuk është datë kalendarike e vlefshme ({})",
                claim.target_value
            )));
        }
        let msg = AttestationMessage {
            value: claim.target_value,
            timestamp,
            claim_key_id: claim_key_id(target_key),
            wallet,
            payload_digest: claim.payload_digest,
            account_tag: account_tag(&self.tag_key(), issuer.unwrap_or(server_name), account_id),
        };
        let bytes = msg.to_bytes();
        let sig = self.key.sign(&bytes);
        let notary = NotaryKey {
            scheme: crate::eddsa::SCHEME.into(),
            public_key: self.key.public(),
        };
        Ok(NotarizedSession {
            version: 3,
            server_name: server_name.into(),
            target_key: target_key.into(),
            http_response: http_response.into(),
            decimals,
            notary,
            attestation: SignedAttestation {
                message_hex: hex::encode(bytes),
                signature: sig,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notary(tag_secret: Option<[u8; 32]>) -> MockNotary {
        MockNotary {
            key: crate::eddsa::SigningKey::from_hex(include_str!(
                "../../../tests/fixtures/mock_notary_seed.hex"
            ))
            .unwrap(),
            tag_secret,
        }
    }

    fn req<'a>(http: &'a str, key: &'a str, issuer: Option<&'a str>) -> NotarizeRequest<'a> {
        NotarizeRequest {
            server_name: "api.banka.al",
            issuer,
            http_response: http,
            target_key: key,
            account_id: "AL35202111090000000001234567",
            wallet: [1; 32],
            timestamp: 1_800_000_000,
            decimals: 0,
        }
    }

    fn tag(s: &NotarizedSession) -> [u8; 32] {
        let m = hex::decode(&s.attestation.message_hex).unwrap();
        AttestationMessage::from_bytes(&m).unwrap().account_tag
    }

    #[test]
    fn stable_tag_secret_and_issuer() {
        let http = sample_bank_response(15_200);
        let a = notary(Some([9; 32])).notarize_with(&req(&http, "balance", Some("banka-x")));
        // një çelës EdDSA tjetër me të njëjtin tag_secret: i njëjti tag (rotacion pa Sybil)
        let other = MockNotary {
            key: crate::eddsa::SigningKey::from_seed([8; 32]),
            tag_secret: Some([9; 32]),
        };
        let b = other.notarize_with(&req(&http, "balance", Some("banka-x")));
        assert_eq!(tag(&a.unwrap()), tag(&b.unwrap()));
        // i njëjti issuer me domen tjetër: i njëjti tag
        let mut r = req(&http, "balance", Some("banka-x"));
        r.server_name = "api2.banka.al";
        let c = notary(Some([9; 32])).notarize_with(&r).unwrap();
        let a = notary(Some([9; 32]))
            .notarize_with(&req(&http, "balance", Some("banka-x")))
            .unwrap();
        assert_eq!(tag(&a), tag(&c));
    }

    #[test]
    fn birth_dates_must_be_calendar_dates() {
        let n = notary(None);
        let ok = sample_identity_response("2001-05-17");
        assert!(n.notarize_with(&req(&ok, "date_of_birth", None)).is_ok());
        let http = |v: &str| {
            let body = format!("{{\"date_of_birth\": {v}, \"id\": \"X1\"}}");
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        for bad in ["20010230", "0", "99999999"] {
            assert!(
                n.notarize_with(&req(&http(bad), "date_of_birth", None))
                    .is_err(),
                "{bad}"
            );
        }
        assert!(n
            .notarize_with(&req(&http("20010517"), "date_of_birth", None))
            .is_ok());
    }

    #[test]
    fn scaled_session_roundtrips_through_prover_check() {
        let body = r#"{"iban":"AL35","balance":15200.99,"currency":"EUR"}"#;
        let http = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let n = notary(None);
        assert!(n.notarize_with(&req(&http, "balance", None)).is_err()); // pa shkallë
        let mut r = req(&http, "balance", None);
        r.decimals = 2;
        let s = n.notarize_with(&r).unwrap();
        let v = s.verify(1_520_099).unwrap();
        assert_eq!(v.message.value, 1_520_099);
    }
}
