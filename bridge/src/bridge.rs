//! Ura: verifikon një prezantim TLSNotary dhe, nëse plotëson politikën, nënshkruan
//! mesazhin kanonik 128-bajtësh me RSA (`NotarizedSession`) që qarku Circom e verifikon.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

use tlsn::{
    attestation::{
        presentation::{Presentation, PresentationOutput},
        CryptoProvider,
    },
    connection::ServerName,
    verifier::ServerCertVerifier,
    webpki::RootCertStore,
};
use zktls_prover::mock::MockNotary;
use zktls_prover::NotarizedSession;

/// Një burim i lejuar (bankë/regjistër) dhe fusha që lexohen prej tij.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Domeni TLS i autentikuar nga noteri.
    pub server: String,
    /// Rruga e kërkesës duhet të fillojë me këtë.
    pub path_prefix: String,
    /// Çelësi JSON i vlerës që provohet (`balance`, `date_of_birth`, ...).
    pub target_key: String,
    /// Çelësi JSON i identifikuesit të llogarisë (burimi i nullifier-it).
    pub account_key: String,
    /// Monedha e detyrueshme (p.sh. `EUR`) për burimet e bilancit: pa këtë,
    /// 15 200 lekë do të kalonin si 15 200 €. Mungon = nuk kontrollohet (p.sh. regjistri i moshës).
    #[serde(default)]
    pub currency: Option<String>,
    /// Çelësi JSON i monedhës në përgjigje.
    #[serde(default = "default_currency_key")]
    pub currency_key: String,
}

fn default_currency_key() -> String {
    "currency".into()
}

/// Politika e urës (skedar JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Çelësat publikë secp256k1 (SEC1 hex) të noterëve të besuar.
    pub notary_keys: Vec<String>,
    pub sources: Vec<Source>,
    /// Mosha maksimale e sesionit TLS.
    #[serde(default = "default_max_age")]
    pub max_age_secs: u64,
}

fn default_max_age() -> u64 {
    600
}

/// Kërkesa te `POST /attest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestRequest {
    /// `bincode(Presentation)` në base64.
    pub presentation: String,
    /// Portofoli Solana (base58) me të cilin lidhet atestimi.
    pub wallet: String,
}

pub struct Bridge {
    pub policy: Policy,
    /// Nënshkruesi RSA (i njëjti format si noteri i simuluar).
    pub signer: MockNotary,
    /// Verifikuesi i certifikatave të serverëve (në prodhim: Mozilla).
    pub roots: RootCertStore,
}

/// Kthen indeksin e fundit të rreshtit të parë (`\r\n`) dhe rrugën e kërkesës.
fn request_path(sent: &[u8]) -> Result<(usize, String)> {
    let end = sent
        .windows(2)
        .position(|w| w == b"\r\n")
        .context("kërkesa nuk ka rresht të parë")?;
    let line = std::str::from_utf8(&sent[..end])?;
    let mut parts = line.split(' ');
    let (method, path) = (parts.next(), parts.next());
    ensure!(method == Some("GET"), "lejohen vetëm kërkesa GET");
    Ok((end, path.context("mungon rruga")?.to_string()))
}

/// Rruga duhet të jetë e thjeshtë: pa `..`, pa `%` (kodim), pa `//`, pa `\\`, vetëm
/// karaktere të sigurta. Përndryshe `/v1/accounts/../loans/me` do të kalonte prefiksin.
fn check_path(path: &str) -> Result<()> {
    ensure!(path.starts_with('/'), "rruga duhet të fillojë me /");
    let (p, query) = path.split_once('?').unwrap_or((path, ""));
    let ok = |c: char| c.is_ascii_alphanumeric() || "/-_.~".contains(c);
    ensure!(
        p.chars().all(ok) && query.chars().all(|c| ok(c) || "=&".contains(c)),
        "rruga ka karaktere të palejuara (p.sh. % ose \\)"
    );
    ensure!(!p.contains("//"), "rruga ka // ");
    ensure!(
        !p.split('/').any(|seg| seg == "." || seg == ".."),
        "rruga ka segmente . ose .."
    );
    Ok(())
}

/// Portofoli i shkruar nga prover-i në header-in [`WALLET_HEADER`] të kërkesës.
/// Rreshti i header-it duhet të jetë i plotë në pjesën e autentikuar të transkriptit.
fn bound_wallet(sent: &[u8], unauthed: &[std::ops::Range<usize>]) -> Result<String> {
    let head_end = sent
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("kërkesa nuk ka fund header-ash")?;
    let mut found = Vec::new();
    let mut start = 0;
    while start < head_end {
        let end = sent[start..head_end]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|i| start + i)
            .unwrap_or(head_end);
        let line = &sent[start..end];
        if let Some(colon) = line.iter().position(|&b| b == b':') {
            if line[..colon].eq_ignore_ascii_case(crate::WALLET_HEADER.as_bytes()) {
                ensure!(
                    unauthed.iter().all(|r| r.end <= start || r.start >= end),
                    "header-i i portofolit nuk është i autentikuar"
                );
                found.push(std::str::from_utf8(&line[colon + 1..])?.trim().to_string());
            }
        }
        start = end + 2;
    }
    ensure!(
        found.len() == 1,
        "sesioni TLS duhet të ketë saktësisht një header `{}` ({})",
        crate::WALLET_HEADER,
        found.len()
    );
    Ok(found.remove(0))
}

/// Vlera (string ose numër) e një çelësi JSON që shfaqet saktësisht një herë.
fn unique_json_value(body: &str, key: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(body).context("trupi nuk është JSON")?;
    let needle = format!("\"{key}\"");
    let count = body
        .match_indices(&needle)
        .filter(|(i, _)| body[i + needle.len()..].trim_start().starts_with(':'))
        .count();
    ensure!(
        count == 1,
        "çelësi `{key}` duhet të shfaqet saktësisht një herë ({count})"
    );
    fn find<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => {
                m.get(key).or_else(|| m.values().find_map(|x| find(x, key)))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|x| find(x, key)),
            _ => None,
        }
    }
    match find(&v, key).context("çelësi mungon")? {
        serde_json::Value::String(s) if !s.is_empty() => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        _ => bail!("çelësi `{key}` duhet të jetë string ose numër"),
    }
}

impl Bridge {
    pub fn attest(&self, req: &AttestRequest, now: u64) -> Result<NotarizedSession> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&req.presentation)
            .context("prezantimi nuk është base64")?;
        let presentation: Presentation =
            bincode::deserialize(&bytes).context("prezantim i pavlefshëm")?;
        self.attest_presentation(presentation, &req.wallet, now)
    }

    pub fn attest_presentation(
        &self,
        presentation: Presentation,
        wallet: &str,
        now: u64,
    ) -> Result<NotarizedSession> {
        // 1. noteri i besuar
        let key = presentation.verifying_key();
        let key_hex = hex::encode(&key.data);
        ensure!(
            self.policy
                .notary_keys
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&key_hex)),
            "noter i panjohur: {key_hex}"
        );

        // 2. nënshkrimi i noterit, certifikata e serverit, angazhimet e transkriptit
        let provider = CryptoProvider {
            cert: ServerCertVerifier::new(&self.roots)?,
            ..Default::default()
        };
        let PresentationOutput {
            server_name,
            connection_info,
            transcript,
            ..
        } = presentation
            .verify(&provider)
            .map_err(|e| anyhow!("prezantimi nuk verifikohet: {e}"))?;
        let server = match server_name.context("prezantimi nuk zbulon domenin")? {
            ServerName::Dns(n) => n.as_str().to_string(),
        };
        let transcript = transcript.context("prezantimi nuk ka transkript")?;

        // 3. freskia
        let t = connection_info.time;
        ensure!(t <= now + 60, "sesion në të ardhmen");
        ensure!(
            now.saturating_sub(t) <= self.policy.max_age_secs,
            "sesioni është më i vjetër se {} s",
            self.policy.max_age_secs
        );

        // 4. burimi i lejuar: domeni + rruga (rreshti i kërkesës duhet të jetë i autentikuar)
        let sent = transcript.sent_unsafe();
        let (line_end, path) = request_path(sent)?;
        if let Some(first_hidden) = transcript.sent_unauthed().min() {
            ensure!(
                first_hidden >= line_end,
                "rreshti i kërkesës nuk është zbuluar"
            );
        }
        check_path(&path)?;
        // portofoli vjen nga sesioni TLS, jo nga kërkesa JSON (anti ri-lidhje)
        let unauthed: Vec<_> = transcript.sent_unauthed().iter().collect();
        let session_wallet = bound_wallet(sent, &unauthed)?;
        ensure!(
            session_wallet == wallet,
            "portofoli i kërkesës ({wallet}) nuk është ai i sesionit TLS ({session_wallet})"
        );
        let source = self
            .policy
            .sources
            .iter()
            .find(|s| s.server == server && path.starts_with(&s.path_prefix))
            .with_context(|| format!("burim i palejuar: {server}{path}"))?;

        // 5. përgjigjja duhet të jetë e zbuluar e plotë (nuk lejohen fusha të fshehura)
        ensure!(
            transcript.received_unauthed().is_empty(),
            "përgjigjja nuk është zbuluar e plotë"
        );
        let response = std::str::from_utf8(transcript.received_unsafe())
            .context("përgjigjja nuk është UTF-8")?;
        let (head, body) = response
            .split_once("\r\n\r\n")
            .context("përgjigje HTTP e pavlefshme")?;
        ensure!(
            head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"),
            "statusi nuk është 200"
        );
        ensure!(
            !head.to_ascii_lowercase().contains("transfer-encoding:"),
            "përgjigjet chunked nuk mbështeten ende"
        );
        let account_id = unique_json_value(body, &source.account_key)?;
        if let Some(expected) = &source.currency {
            let got = unique_json_value(body, &source.currency_key)
                .with_context(|| format!("mungon monedha (`{}`)", source.currency_key))?;
            ensure!(
                got.eq_ignore_ascii_case(expected),
                "monedhë e gabuar: {got} (pritet {expected})"
            );
        }

        // 6. nënshkruaj mesazhin kanonik (normalizuesi refuzon dublikatat/escape)
        let wallet: [u8; 32] = bs58_decode(wallet)?;
        Ok(self.signer.notarize(
            &server,
            response,
            &source.target_key,
            &account_id,
            wallet,
            t,
        )?)
    }
}

fn bs58_decode(s: &str) -> Result<[u8; 32]> {
    bs58::decode(s)
        .into_vec()
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v).ok())
        .context("portofol i pavlefshëm (pritet çelës publik Solana base58)")
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_values() {
        let b = r#"{"iban":"AL35","balance":15200,"x":{"pid":12}}"#;
        assert_eq!(unique_json_value(b, "iban").unwrap(), "AL35");
        assert_eq!(unique_json_value(b, "pid").unwrap(), "12");
        assert!(unique_json_value(r#"{"iban":"A","y":{"iban":"B"}}"#, "iban").is_err());
        assert!(unique_json_value(r#"{"iban":""}"#, "iban").is_err());
    }

    #[test]
    fn paths() {
        assert!(check_path("/v1/accounts/me").is_ok());
        assert!(check_path("/v1/accounts/me?fields=balance&x=1").is_ok());
        for bad in [
            "/v1/accounts/../loans/me",
            "/v1/accounts/%2e%2e/loans/me",
            "/v1/accounts//me",
            "/v1/accounts/./me",
            "/v1/accounts\\..\\loans",
            "v1/accounts/me",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wallet_header() {
        let req = b"GET /v1/me HTTP/1.1\r\nhost: x\r\nx-zktls-wallet: W1\r\n\r\n";
        assert_eq!(bound_wallet(req, &[]).unwrap(), "W1");
        // i fshehur (i paautentikuar) → refuzohet
        assert!(bound_wallet(req, std::slice::from_ref(&(30..40))).is_err());
        // mungon ose dy herë → refuzohet
        assert!(bound_wallet(b"GET / HTTP/1.1\r\nhost: x\r\n\r\n", &[]).is_err());
        let two = b"GET / HTTP/1.1\r\nX-Zktls-Wallet: A\r\nx-zktls-wallet: B\r\n\r\n";
        assert!(bound_wallet(two, &[]).is_err());
    }

    #[test]
    fn request_line() {
        let (end, p) = request_path(b"GET /v1/me HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!((end, p.as_str()), (19, "/v1/me"));
        assert!(request_path(b"POST /v1/me HTTP/1.1\r\n").is_err());
    }
}
