//! Ura: verifikon një prezantim TLSNotary dhe, nëse plotëson politikën, nënshkruan
//! mesazhin kanonik 128-bajtësh me EdDSA (`NotarizedSession`) që qarku Circom e verifikon.

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
    #[serde(default = "default_method")]
    pub method: String,
    /// Exact, public, read-only POST query. Dynamic/secret bodies are unsupported.
    #[serde(default)]
    pub request_body: Option<String>,
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
    /// Lejon query string (`?...`). Parazgjedhur jo: një parametër si `?as_of=2019-01-01`
    /// ose `?account=...` mund të ndryshonte kuptimin e përgjigjes pa ndryshuar rrugën.
    #[serde(default)]
    pub allow_query: bool,
    /// JSON Pointer (RFC 6901) i vlerës, p.sh. `/account/balance`. Pa të, çelësi
    /// pranohet kudo në JSON (vetëm një herë); me të, duhet të jetë pikërisht aty, që
    /// p.sh. një `loan.balance` i vetëm të mos lexohet si bilanc.
    #[serde(default)]
    pub target_pointer: Option<String>,
    /// JSON Pointer i identifikuesit të llogarisë, p.sh. `/account/iban`.
    #[serde(default)]
    pub account_pointer: Option<String>,
    /// Shkalla dhjetore e vlerës: 0 = njësi të plota (`15200.99` refuzohet), 2 = centë
    /// (`15200.99` → `1520099`). Pragjet on-chain janë në të njëjtën njësi.
    #[serde(default)]
    pub decimals: u8,
    /// Identiteti i qëndrueshëm i lëshuesit për nullifier-in (p.sh. `"banka-x"`): i
    /// njëjtë për të gjitha domenet e bankës dhe pas ndërrimit të domenit. Pa të,
    /// përdoret domeni TLS.
    #[serde(default)]
    pub issuer: Option<String>,
}

fn default_currency_key() -> String {
    "currency".into()
}

fn default_method() -> String {
    "GET".into()
}

impl Source {
    /// Rruga (pa query) i përket burimit: prefiksi përputhet në kufi segmenti, pra
    /// `/v1/accounts` pranon `/v1/accounts/me`, jo `/v1/accounts-archive/me`.
    fn matches_path(&self, path: &str) -> bool {
        let p = path.split_once('?').map_or(path, |(p, _)| p);
        match p.strip_prefix(self.path_prefix.as_str()) {
            Some(rest) => {
                self.path_prefix.ends_with('/') || rest.is_empty() || rest.starts_with('/')
            }
            None => false,
        }
    }
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

impl Policy {
    /// Kontrollet e politikës në nisje (jo në çdo kërkesë).
    ///
    /// Të gjitha burimet nënshkruhen me të njëjtin çelës EdDSA, pra on-chain duken si një
    /// burim i vetëm: dy burime "balance" me monedha të ndryshme (EUR dhe ALL) do të
    /// bënin "bilanc ≥ 5 000" të paqartë. Prandaj burimet e të njëjtit `target_key`
    /// duhet të kenë të njëjtën monedhë.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.notary_keys.is_empty(),
            "politika nuk ka noterë të besuar"
        );
        ensure!(!self.sources.is_empty(), "politika nuk ka burime");
        ensure!(self.max_age_secs > 0, "max_age_secs duhet të jetë > 0");
        for s in &self.sources {
            ensure!(
                s.method == "GET" || s.method == "POST",
                "source method must be GET or POST"
            );
            if s.method == "POST" {
                let body = s
                    .request_body
                    .as_deref()
                    .context("POST source requires an exact request_body")?;
                ensure!(
                    body.len() <= crate::MAX_SENT_DATA / 2,
                    "POST policy body too large"
                );
                ensure!(
                    serde_json::from_str::<serde_json::Value>(body)?.is_object(),
                    "POST body must be a JSON object"
                );
            } else {
                ensure!(
                    s.request_body.is_none(),
                    "GET source cannot have request_body"
                );
            }
            ensure!(!s.server.is_empty(), "burim pa server");
            check_path(&s.path_prefix)
                .with_context(|| format!("path_prefix `{}`", s.path_prefix))?;
            ensure!(
                !s.path_prefix.contains('?'),
                "path_prefix `{}` nuk mund të ketë query",
                s.path_prefix
            );
            ensure!(
                !s.target_key.is_empty() && !s.account_key.is_empty(),
                "burimi {} ka target_key/account_key bosh",
                s.server
            );
            ensure!(
                s.decimals <= triad_core::MAX_DECIMALS,
                "decimals > {}",
                triad_core::MAX_DECIMALS
            );
            ensure!(
                s.decimals == 0
                    || !zktls_prover::witness::BIRTH_DATE_KEYS.contains(&s.target_key.as_str()),
                "një datë lindjeje nuk ka decimale"
            );
            if let Some(i) = &s.issuer {
                ensure!(!i.trim().is_empty(), "issuer bosh te {}", s.server);
            }
            for (ptr, key) in [
                (&s.target_pointer, &s.target_key),
                (&s.account_pointer, &s.account_key),
            ] {
                if let Some(p) = ptr {
                    ensure!(
                        p.starts_with('/') && pointer_last(p).as_deref() == Some(key.as_str()),
                        "pointer `{p}` duhet të fillojë me / dhe të mbarojë me `{key}`"
                    );
                }
            }
            let cur = |x: &Source| x.currency.as_deref().map(str::to_ascii_uppercase);
            for o in self.sources.iter().filter(|o| o.target_key == s.target_key) {
                ensure!(
                    cur(o) == cur(s),
                    "burimet e `{}` kanë monedha të ndryshme ({:?} te {}, {:?} te {})",
                    s.target_key,
                    s.currency,
                    s.server,
                    o.currency,
                    o.server
                );
                ensure!(
                    o.decimals == s.decimals,
                    "burimet e `{}` kanë shkallë dhjetore të ndryshme ({} dhe {})",
                    s.target_key,
                    s.decimals,
                    o.decimals
                );
            }
        }
        Ok(())
    }
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
    /// Nënshkruesi EdDSA (i njëjti format si noteri i simuluar).
    pub signer: MockNotary,
    /// Verifikuesi i certifikatave të serverëve (në prodhim: Mozilla).
    pub roots: RootCertStore,
}

fn check_response_encoding(head: &str) -> Result<()> {
    for (name, value) in head.split("\r\n").filter_map(|l| l.split_once(':')) {
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            anyhow::bail!("chunked/transfer-encoded responses are unsupported");
        }
        if name.trim().eq_ignore_ascii_case("content-encoding") {
            ensure!(
                value.trim().eq_ignore_ascii_case("identity"),
                "compressed responses are unsupported; server must honor Accept-Encoding: identity"
            );
        }
    }
    Ok(())
}

fn response_parts(raw: &[u8]) -> Result<(&str, &str)> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("invalid HTTP response")?;
    let head = std::str::from_utf8(&raw[..end]).context("invalid HTTP headers")?;
    // Reject encodings before attempting to decode a potentially binary body.
    check_response_encoding(head)?;
    let body = std::str::from_utf8(&raw[end + 4..]).context("response body is not UTF-8")?;
    Ok((head, body))
}

/// Kthen indeksin e fundit të rreshtit të parë (`\r\n`) dhe rrugën e kërkesës.
fn request_path(sent: &[u8]) -> Result<(usize, String, String)> {
    let end = sent
        .windows(2)
        .position(|w| w == b"\r\n")
        .context("kërkesa nuk ka rresht të parë")?;
    let line = std::str::from_utf8(&sent[..end])?;
    let mut parts = line.split(' ');
    let (method, path) = (parts.next(), parts.next());
    ensure!(
        matches!(method, Some("GET" | "POST")),
        "only GET and POST are supported"
    );
    ensure!(
        matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1")) && parts.next().is_none(),
        "invalid HTTP request line"
    );
    Ok((
        end,
        method.unwrap().to_string(),
        path.context("mungon rruga")?.to_string(),
    ))
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

/// Validate one authenticated request and its exact policy-approved framing/body.
fn validate_request(
    sent: &[u8],
    unauthed: &[std::ops::Range<usize>],
    source: &Source,
) -> Result<()> {
    let head_end = sent
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("kërkesa nuk ka fund header-ash")?;
    // një GET i vetëm pa trup: asgjë pas header-ave (p.sh. një kërkesë e dytë e fshehur)
    let authenticated =
        |start: usize, end: usize| unauthed.iter().all(|r| r.end <= start || r.start >= end);
    ensure!(
        authenticated(head_end, head_end + 4),
        "hidden HTTP delimiter"
    );
    let mut lengths = Vec::new();
    let mut hosts = Vec::new();
    let mut types = Vec::new();
    let mut start = sent
        .windows(2)
        .position(|w| w == b"\r\n")
        .context("missing request line")?
        + 2;
    while start < head_end {
        let end = sent[start..head_end]
            .windows(2)
            .position(|w| w == b"\r\n")
            .map(|i| start + i)
            .unwrap_or(head_end);
        let line = &sent[start..end];
        let colon = line
            .iter()
            .position(|&b| b == b':')
            .context("invalid HTTP header")?;
        let name = hyper::header::HeaderName::from_bytes(&line[..colon])?;
        ensure!(
            authenticated(start, start + colon + 1) && authenticated(end, end + 2),
            "hidden HTTP framing"
        );
        if !crate::SECRET_HEADERS.contains(&name.as_str()) {
            ensure!(
                authenticated(start, end),
                "non-secret HTTP header must be authenticated"
            );
        }
        let value = std::str::from_utf8(&line[colon + 1..])?.trim();
        match name.as_str() {
            "transfer-encoding" => bail!("request Transfer-Encoding is unsupported"),
            "content-length" => lengths.push(value.parse::<usize>()?),
            "host" => hosts.push(value),
            "content-type" => types.push(value),
            _ => (),
        }
        start = end + 2;
    }
    ensure!(
        hosts.len() == 1
            && (hosts[0].eq_ignore_ascii_case(&source.server)
                || hosts[0].eq_ignore_ascii_case(&format!("{}:443", source.server))),
        "Host must match the source domain"
    );
    ensure!(lengths.len() <= 1, "duplicate request Content-Length");
    let body = &sent[head_end + 4..];
    if let Some(len) = lengths.first() {
        ensure!(
            *len == body.len(),
            "request body length mismatch or pipelined request"
        );
    }
    if source.method == "GET" {
        ensure!(
            body.is_empty(),
            "GET must not have a body or pipelined request"
        );
    } else {
        ensure!(
            lengths.len() == 1
                && types.len() == 1
                && types[0]
                    .split(';')
                    .next()
                    .unwrap()
                    .trim()
                    .eq_ignore_ascii_case("application/json"),
            "POST requires JSON and Content-Length"
        );
        ensure!(
            authenticated(head_end + 4, sent.len()),
            "POST body must be authenticated"
        );
        ensure!(
            source.request_body.as_deref().map(str::as_bytes) == Some(body),
            "POST body does not match source policy"
        );
    }
    Ok(())
}

/// Segmenti i fundit i një JSON Pointer-i (me `~1` → `/`, `~0` → `~`).
fn pointer_last(p: &str) -> Option<String> {
    let seg = p.rsplit('/').next()?;
    Some(seg.replace("~1", "/").replace("~0", "~"))
}

/// Çelësi unik ndodhet pikërisht te `pointer` (nëse politika e jep).
fn check_pointer(body: &str, pointer: Option<&str>, key: &str) -> Result<()> {
    let Some(p) = pointer else { return Ok(()) };
    let v: serde_json::Value = serde_json::from_str(body).context("trupi nuk është JSON")?;
    ensure!(
        v.pointer(p).is_some(),
        "`{key}` nuk është te {p} (struktura e përgjigjes nuk është ajo e pritura)"
    );
    Ok(())
}

/// Madhësia maksimale e transkriptit (secili drejtim) që pranon ura. Klienti ynë
/// kufizohet në 4 KiB / 16 KiB; kufiri këtu mbron memorien.
pub const MAX_TRANSCRIPT_BYTES: usize = 1 << 20;

/// Gjatësitë (dërguar, marrë) që deklaron transkripti i prezantimit, të lexuara PA
/// deserializimin e plotë: `PartialTranscript` alokon `vec![0; sent_total]` dhe
/// `vec![0; recv_total]` me vlera që i jep klienti, pra 1 MiB kërkesë mund të kërkonte
/// terabajtë memorie para se të kontrollohet nënshkrimi i noterit. Struktura hije ka
/// të njëjtën renditje fushash si `Presentation` (bincode nuk ka emra fushash), por
/// pa alokimin.
pub fn declared_transcript_sizes(bytes: &[u8]) -> Result<Option<(usize, usize)>> {
    use tlsn::rangeset::set::RangeSet;
    #[derive(Deserialize)]
    struct Compressed {
        _sent_authed: Vec<u8>,
        _received_authed: Vec<u8>,
        _sent_idx: RangeSet<usize>,
        _recv_idx: RangeSet<usize>,
        sent_total: usize,
        recv_total: usize,
    }
    #[derive(Deserialize)]
    struct TranscriptProof {
        transcript: Compressed,
        _hash_secrets: Vec<tlsn::transcript::hash::PlaintextHashSecret>,
    }
    #[derive(Deserialize)]
    struct Shadow {
        _attestation: tlsn::attestation::AttestationProof,
        _identity: Option<tlsn::attestation::connection::ServerIdentityProof>,
        transcript: Option<TranscriptProof>,
    }
    let s: Shadow = bincode::deserialize(bytes).context("prezantim i pavlefshëm")?;
    Ok(s.transcript
        .map(|t| (t.transcript.sent_total, t.transcript.recv_total)))
}

fn check_transcript_sizes(bytes: &[u8]) -> Result<()> {
    if let Some((sent, recv)) = declared_transcript_sizes(bytes)? {
        ensure!(
            sent <= MAX_TRANSCRIPT_BYTES && recv <= MAX_TRANSCRIPT_BYTES,
            "transkripti deklaron {sent} / {recv} bajte (> {MAX_TRANSCRIPT_BYTES} B)"
        );
    }
    Ok(())
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
    // edhe në pemën e analizuar: një çelës i shkruar me escape (`"\u0069ban"`) nuk
    // shfaqet në numërimin e tekstit, por serde e njeh si `iban`
    fn occurrences(v: &serde_json::Value, key: &str) -> usize {
        match v {
            serde_json::Value::Object(m) => m
                .iter()
                .map(|(k, x)| usize::from(k == key) + occurrences(x, key))
                .sum(),
            serde_json::Value::Array(a) => a.iter().map(|x| occurrences(x, key)).sum(),
            _ => 0,
        }
    }
    ensure!(
        occurrences(&v, key) == 1,
        "çelësi `{key}` duhet të shfaqet saktësisht një herë në JSON"
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
        check_transcript_sizes(&bytes)?;
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
            extensions,
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
        let (line_end, method, path) = request_path(sent)?;
        if let Some(first_hidden) = transcript.sent_unauthed().min() {
            ensure!(
                first_hidden >= line_end,
                "rreshti i kërkesës nuk është zbuluar"
            );
        }
        check_path(&path)?;
        // portofoli vjen nga sesioni TLS, jo nga kërkesa JSON (anti ri-lidhje)
        let unauthed: Vec<_> = transcript.sent_unauthed().iter().collect();
        let session_wallet = crate::wallet_from_extensions(&extensions)?;
        ensure!(
            session_wallet == wallet,
            "portofoli i kërkesës ({wallet}) nuk është ai i sesionit TLS ({session_wallet})"
        );
        let source = self
            .policy
            .sources
            .iter()
            .find(|s| s.server == server && s.method == method && s.matches_path(&path))
            .with_context(|| format!("burim i palejuar: {server}{path}"))?;
        validate_request(sent, &unauthed, source)?;
        ensure!(
            source.allow_query || !path.contains('?'),
            "ky burim nuk lejon query string ({path})"
        );

        // 5. përgjigjja duhet të jetë e zbuluar e plotë (nuk lejohen fusha të fshehura)
        ensure!(
            transcript.received_unauthed().is_empty(),
            "përgjigjja nuk është zbuluar e plotë"
        );
        let raw_response = transcript.received_unsafe();
        let (head, body) = response_parts(raw_response)?;
        let response = std::str::from_utf8(raw_response).expect("validated header and body");
        ensure!(
            head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"),
            "statusi nuk është 200"
        );
        // një përgjigje e vetme: trupi është saktësisht Content-Length bajte
        let lengths: Vec<&str> = head
            .split("\r\n")
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
            .map(|(_, v)| v.trim())
            .collect();
        ensure!(lengths.len() <= 1, "përgjigjja ka disa Content-Length");
        if let Some(cl) = lengths.first() {
            let cl: usize = cl.parse().context("Content-Length i pavlefshëm")?;
            ensure!(
                cl == body.len(),
                "trupi ka {} bajte, Content-Length thotë {cl} (përgjigje e dytë?)",
                body.len()
            );
        }
        let account_id = unique_json_value(body, &source.account_key)?;
        check_pointer(body, source.target_pointer.as_deref(), &source.target_key)?;
        check_pointer(body, source.account_pointer.as_deref(), &source.account_key)?;
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
        Ok(self
            .signer
            .notarize_with(&zktls_prover::mock::NotarizeRequest {
                server_name: &server,
                issuer: source.issuer.as_deref(),
                http_response: response,
                target_key: &source.target_key,
                account_id: &account_id,
                wallet,
                timestamp: t,
                decimals: source.decimals,
            })?)
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
    fn signed_wallet_extension() {
        let wallet = "11111111111111111111111111111111";
        let ext = crate::wallet_extension(wallet).unwrap();
        assert_eq!(
            crate::wallet_from_extensions(std::slice::from_ref(&ext)).unwrap(),
            wallet
        );
        assert!(crate::wallet_from_extensions(&[]).is_err());
        assert!(crate::wallet_from_extensions(&[ext.clone(), ext.clone()]).is_err());
        let mut wrong = ext;
        wrong.value.pop();
        assert!(crate::wallet_from_extensions(&[wrong]).is_err());
        assert!(crate::wallet_extension("invalid").is_err());
    }

    #[test]
    fn single_request_only() {
        let source = src("/v1/me");
        let good = b"GET /v1/me HTTP/1.1\r\nhost: api.banka.al\r\n\r\n";
        assert!(validate_request(good, &[], &source).is_ok());
        assert!(validate_request(good, std::slice::from_ref(&(30..34)), &source).is_err());
        for suffix in [b"x".as_slice(), b"GET /v1/loans HTTP/1.1\r\n\r\n"] {
            let mut bad = good.to_vec();
            bad.extend_from_slice(suffix);
            assert!(validate_request(&bad, &[], &source).is_err());
        }
    }

    #[test]
    fn exact_post_policy_rejects_hidden_modified_or_smuggled_body() {
        let mut source = src("/v1/me");
        source.method = "POST".into();
        source.request_body = Some("{}".into());
        let good = b"POST /v1/me HTTP/1.1\r\nhost: api.banka.al\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
        assert!(validate_request(good, &[], &source).is_ok());
        assert!(validate_request(
            good,
            std::slice::from_ref(&(good.len() - 2..good.len())),
            &source
        )
        .is_err());
        let mut changed = good.to_vec();
        let n = changed.len();
        changed[n - 1] = b']';
        assert!(validate_request(&changed, &[], &source).is_err());
        let duplicate = String::from_utf8(good.to_vec()).unwrap().replace(
            "content-length: 2",
            "content-length: 2\r\ncontent-length: 2",
        );
        assert!(validate_request(duplicate.as_bytes(), &[], &source).is_err());
        let te = duplicate.replace(
            "content-length: 2\r\ncontent-length: 2",
            "transfer-encoding: chunked",
        );
        assert!(validate_request(te.as_bytes(), &[], &source).is_err());
    }

    #[test]
    fn escaped_duplicate_key() {
        // teksti ka një `"iban"`, por serde sheh dy (njëri me escape)
        let b = r#"{"\u0069ban":"X","y":{"iban":"Y"}}"#;
        assert!(unique_json_value(b, "iban").is_err());
    }

    fn src(prefix: &str) -> Source {
        Source {
            method: "GET".into(),
            request_body: None,
            server: "api.banka.al".into(),
            path_prefix: prefix.into(),
            target_key: "balance".into(),
            account_key: "iban".into(),
            currency: Some("EUR".into()),
            currency_key: "currency".into(),
            allow_query: false,
            target_pointer: None,
            account_pointer: None,
            decimals: 0,
            issuer: None,
        }
    }

    #[test]
    fn example_policy_is_valid() {
        let p: Policy =
            serde_json::from_str(include_str!("../bridge-policy.example.json")).unwrap();
        p.validate().unwrap();
    }

    #[test]
    fn json_pointer() {
        let b = r#"{"account":{"iban":"AL35","balance":15200},"loan":{"amount":5}}"#;
        assert!(check_pointer(b, Some("/account/balance"), "balance").is_ok());
        let loan = r#"{"account":{"iban":"AL35"},"loan":{"balance":50000}}"#;
        assert!(check_pointer(loan, Some("/account/balance"), "balance").is_err());
        assert!(check_pointer(loan, None, "balance").is_ok());
        assert_eq!(pointer_last("/a/b~1c").as_deref(), Some("b/c"));
    }

    #[test]
    fn rejects_huge_declared_transcript_before_allocating() {
        assert!(check_transcript_sizes(b"\x00\x01").is_err()); // jo prezantim
    }

    #[test]
    fn prefix_on_segment_boundary() {
        let s = src("/v1/accounts");
        assert!(s.matches_path("/v1/accounts"));
        assert!(s.matches_path("/v1/accounts/me"));
        assert!(s.matches_path("/v1/accounts/me?x=1"));
        assert!(!s.matches_path("/v1/accounts-archive/me"));
        assert!(!s.matches_path("/v1/accountsX"));
        let s = src("/v1/accounts/");
        assert!(s.matches_path("/v1/accounts/me"));
        assert!(!s.matches_path("/v1/accounts"));
    }

    #[test]
    fn policy_validation() {
        let p = |sources: Vec<Source>| Policy {
            notary_keys: vec!["02ab".into()],
            sources,
            max_age_secs: 600,
        };
        assert!(p(vec![src("/v1/accounts/")]).validate().is_ok());
        let mut all = src("/v2/accounts/");
        all.currency = Some("ALL".into());
        assert!(p(vec![src("/v1/accounts/"), all.clone()])
            .validate()
            .is_err());
        all.currency = Some("eur".into());
        assert!(p(vec![src("/v1/accounts/"), all]).validate().is_ok());
        assert!(p(vec![src("/v1/../x/")]).validate().is_err());
        let mut cents = src("/v2/accounts/");
        cents.decimals = 2;
        assert!(p(vec![src("/v1/accounts/"), cents]).validate().is_err());
        let mut ptr = src("/v1/accounts/");
        ptr.target_pointer = Some("/account/amount".into());
        assert!(p(vec![ptr]).validate().is_err());
        assert!(p(vec![]).validate().is_err());
    }

    #[test]
    fn request_line() {
        let (end, method, p) = request_path(b"GET /v1/me HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert_eq!((end, method.as_str(), p.as_str()), (19, "GET", "/v1/me"));
        assert!(request_path(b"POST /v1/me HTTP/1.1\r\n").is_ok());
        assert!(request_path(b"PUT /v1/me HTTP/1.1\r\n").is_err());
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[test]
    fn accepts_identity_and_refuses_server_encoding_overrides() {
        assert!(check_response_encoding("HTTP/1.1 200 OK\r\nContent-Length: 2").is_ok());
        assert!(check_response_encoding("HTTP/1.1 200 OK\r\nContent-Encoding: Identity").is_ok());
        for line in [
            "Content-Encoding: gzip",
            "content-encoding: br",
            "Content-Encoding: gzip, identity",
            "Transfer-Encoding: chunked",
            "transfer-encoding: identity",
        ] {
            assert!(
                check_response_encoding(&format!("HTTP/1.1 200 OK\r\n{line}")).is_err(),
                "{line}"
            );
        }
        let binary_gzip = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n\x1f\x8b\xff";
        assert!(response_parts(binary_gzip)
            .unwrap_err()
            .to_string()
            .contains("compressed"));
        assert_eq!(
            response_parts(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap()
                .1,
            "{}"
        );
    }
}
