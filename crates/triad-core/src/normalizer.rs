//! MED-01: Al-Jabr – thjeshtimi kanonik i të dhënave Web2.
//!
//! Merr përgjigjen e papërpunuar HTTP (headers + trup JSON) dhe:
//! 1. gjen fushat e ndjeshme (emër, IBAN, cookie, …) pa kopjuar memorie;
//! 2. nxjerr vlerën e pretenduar (p.sh. `"balance": 15200`) me parsim zero-copy;
//! 3. "balancon" shprehjen `value − threshold ≥ 0` dhe kthen [`CanonicalClaim`].
//!
//! Asnjë funksion këtu nuk alokon në heap: intervalet e maskimit mbahen në një
//! bufer fiks në stack ([`MAX_MASKED_FIELDS`]) dhe hash-i i ngarkesës së maskuar
//! llogaritet në rrjedhë (streaming), pa ndërtuar kopje.

use sha2::{Digest, Sha256};
use subtle::{ConditionallySelectable, ConstantTimeEq};

/// Çelësat JSON që konsiderohen të dhëna personale (krahasim pa dallim shkronjash).
pub const SENSITIVE_JSON_KEYS: &[&str] = &[
    "name",
    "first_name",
    "firstname",
    "last_name",
    "lastname",
    "surname",
    "full_name",
    "account_number",
    "account",
    "iban",
    "email",
    "phone",
    "address",
    "personal_id",
];

/// Header-at HTTP që maskohen.
pub const SENSITIVE_HEADERS: &[&str] = &["set-cookie", "cookie", "authorization", "x-session-id"];

/// Numri maksimal i fushave të maskuara në një ngarkesë.
pub const MAX_MASKED_FIELDS: usize = 64;

const DOMAIN_TAG: &[u8] = b"triad-core/MED-01/claim/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizerError {
    /// Përgjigja HTTP nuk ka ndarësin `\r\n\r\n`.
    MissingBody,
    /// Çelësi i synuar nuk u gjet.
    KeyNotFound,
    /// Çelësi i synuar shfaqet më shumë se një herë (paqartësi → refuzim).
    DuplicateKey,
    /// Vlera nuk është numër i plotë jo-negativ (lejohet pjesë dhjetore, hidhet).
    InvalidNumber,
    /// Vlera nuk nxë në u64.
    Overflow,
    /// Çelësi i synuar është bosh ose përmban `"`/`\`.
    InvalidKey,
    /// Më shumë se [`MAX_MASKED_FIELDS`] fusha të ndjeshme.
    TooManySensitiveFields,
}

impl core::fmt::Display for NormalizerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for NormalizerError {}

/// Pretendimi në formë kanonike, gati për qarkun ZK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalClaim {
    pub target_value: u64,
    pub threshold: u64,
    pub is_greater_or_equal: bool,
    /// SHA-256(domain ‖ key ‖ value ‖ threshold ‖ SHA-256(ngarkesa e maskuar)).
    /// Lidh pretendimin me transkriptin e saktë; nuk është zotimi Poseidon on-chain.
    pub commitment_hash: [u8; 32],
    /// SHA-256 i ngarkesës pas maskimit (hyn në mesazhin e atestimit).
    pub payload_digest: [u8; 32],
    /// 8 bajtët e parë të SHA-256(target_key), big-endian.
    pub claim_key_id: u64,
}

impl CanonicalClaim {
    /// `value − threshold` nëse kushti plotësohet.
    pub fn margin(&self) -> Option<u64> {
        self.target_value.checked_sub(self.threshold)
    }
}

/// Interval bajtesh për maskim: `[start, end)` dhe bajti zëvendësues.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct MaskRange {
    pub start: usize,
    pub end: usize,
    pub fill: u8,
}

/// Bufer fiks (stack) me intervalet e maskimit, të renditura sipas `start`.
pub struct MaskRanges {
    ranges: [MaskRange; MAX_MASKED_FIELDS],
    len: usize,
}

impl MaskRanges {
    fn new() -> Self {
        Self {
            ranges: [MaskRange::default(); MAX_MASKED_FIELDS],
            len: 0,
        }
    }
    fn push(&mut self, r: MaskRange) -> Result<(), NormalizerError> {
        if r.start >= r.end {
            return Ok(());
        }
        if self.len == MAX_MASKED_FIELDS {
            return Err(NormalizerError::TooManySensitiveFields);
        }
        self.ranges[self.len] = r;
        self.len += 1;
        Ok(())
    }
    pub fn as_slice(&self) -> &[MaskRange] {
        &self.ranges[..self.len]
    }
}

pub struct AlgebraicNormalizer;

impl AlgebraicNormalizer {
    /// Redukton strukturën e të dhënave Web2 në formë kanonike minimale.
    pub fn normalize_and_mask(
        raw_payload: &[u8],
        target_key: &str,
        threshold: u64,
    ) -> Result<CanonicalClaim, NormalizerError> {
        validate_key(target_key)?;
        let (_, body_off) = split_http(raw_payload)?;
        // 1. Gjen çelësin me zero-copy parsing
        let target_value = find_unique_json_u64(&raw_payload[body_off..], target_key)?;
        // 2. Balancon shprehjen: (value − threshold ≥ 0), pa degëzim
        let (_, borrow) = target_value.overflowing_sub(threshold);
        let ge = u8::conditional_select(&1, &0, (borrow as u8).ct_eq(&1));
        // 3. Hash-i i ngarkesës së maskuar + pretendimi kanonik
        let payload_digest = Self::masked_digest(raw_payload)?;
        let claim_key_id = claim_key_id(target_key);
        let mut h = Sha256::new();
        h.update(DOMAIN_TAG);
        h.update([target_key.len() as u8]);
        h.update(target_key.as_bytes());
        h.update(target_value.to_be_bytes());
        h.update(threshold.to_be_bytes());
        h.update(payload_digest);
        Ok(CanonicalClaim {
            target_value,
            threshold,
            is_greater_or_equal: ge == 1,
            commitment_hash: h.finalize().into(),
            payload_digest,
            claim_key_id,
        })
    }

    /// Gjen intervalet e fushave të ndjeshme (headers + JSON).
    pub fn sensitive_ranges(raw: &[u8]) -> Result<MaskRanges, NormalizerError> {
        let mut out = MaskRanges::new();
        let (header_end, body_off) = split_http(raw)?;
        // Headers: "Name: value\r\n"
        let mut line_start = 0;
        while line_start < header_end {
            let line_end = find(&raw[line_start..header_end], b"\r\n")
                .map(|p| line_start + p)
                .unwrap_or(header_end);
            let line = &raw[line_start..line_end];
            if let Some(colon) = line.iter().position(|&b| b == b':') {
                let name = trim(&line[..colon]);
                if SENSITIVE_HEADERS
                    .iter()
                    .any(|h| name.eq_ignore_ascii_case(h.as_bytes()))
                {
                    let mut vs = colon + 1;
                    while vs < line.len() && line[vs] == b' ' {
                        vs += 1;
                    }
                    out.push(MaskRange {
                        start: line_start + vs,
                        end: line_end,
                        fill: b'*',
                    })?;
                }
            }
            line_start = line_end + 2;
        }
        // Trupi JSON
        let body = &raw[body_off..];
        let mut i = 0;
        while let Some((key, after_colon)) = next_json_key(body, i) {
            i = after_colon;
            let k = &body[key.0..key.1];
            if !SENSITIVE_JSON_KEYS
                .iter()
                .any(|s| k.eq_ignore_ascii_case(s.as_bytes()))
            {
                continue;
            }
            let vs = skip_ws(body, after_colon);
            if vs >= body.len() {
                break;
            }
            if body[vs] == b'"' {
                let end = string_end(body, vs + 1).ok_or(NormalizerError::InvalidNumber)?;
                out.push(MaskRange {
                    start: body_off + vs + 1,
                    end: body_off + end,
                    fill: b'*',
                })?;
                i = end + 1;
            } else {
                let mut e = vs;
                while e < body.len()
                    && (body[e].is_ascii_digit() || body[e] == b'.' || body[e] == b'-')
                {
                    e += 1;
                }
                out.push(MaskRange {
                    start: body_off + vs,
                    end: body_off + e,
                    fill: b'0',
                })?;
                i = e;
            }
        }
        // rendit sipas fillimit (insertion sort – pa alokim)
        let s = &mut out.ranges[..out.len];
        for a in 1..s.len() {
            let mut b = a;
            while b > 0 && s[b - 1].start > s[b].start {
                s.swap(b - 1, b);
                b -= 1;
            }
        }
        Ok(out)
    }

    /// Maskon në vend fushat e ndjeshme (gjatësia ruhet). Kthen numrin e fushave.
    pub fn mask_sensitive_in_place(payload: &mut [u8]) -> Result<usize, NormalizerError> {
        let ranges = Self::sensitive_ranges(payload)?;
        for r in ranges.as_slice() {
            payload[r.start..r.end].fill(r.fill);
        }
        Ok(ranges.len)
    }

    /// SHA-256 i ngarkesës *si do të ishte pas maskimit*, i llogaritur në
    /// rrjedhë pa kopjuar ngarkesën.
    pub fn masked_digest(raw: &[u8]) -> Result<[u8; 32], NormalizerError> {
        let ranges = Self::sensitive_ranges(raw)?;
        let mut h = Sha256::new();
        let mut pos = 0;
        let fill_block = [0u8; 64];
        for r in ranges.as_slice() {
            if r.start < pos {
                continue; // mbivendosje (nuk ndodh në praktikë)
            }
            h.update(&raw[pos..r.start]);
            let mut left = r.end - r.start;
            let mut blk = fill_block;
            blk.fill(r.fill);
            while left > 0 {
                let n = left.min(64);
                h.update(&blk[..n]);
                left -= n;
            }
            pos = r.end;
        }
        h.update(&raw[pos..]);
        Ok(h.finalize().into())
    }
}

/// 8 bajtët e parë të SHA-256(key) si u64 big-endian.
pub fn claim_key_id(key: &str) -> u64 {
    let d = Sha256::digest(key.as_bytes());
    u64::from_be_bytes(d[..8].try_into().expect("8 bajte"))
}

fn validate_key(key: &str) -> Result<(), NormalizerError> {
    if key.is_empty() || key.len() > 255 || key.bytes().any(|b| b == b'"' || b == b'\\' || b < 0x20)
    {
        return Err(NormalizerError::InvalidKey);
    }
    Ok(())
}

/// Kthen (fundi i headers, fillimi i trupit). Nëse ngarkesa nuk fillon me
/// `HTTP/`, e gjithë ngarkesa trajtohet si trup.
fn split_http(raw: &[u8]) -> Result<(usize, usize), NormalizerError> {
    if !raw.starts_with(b"HTTP/") {
        return Ok((0, 0));
    }
    find(raw, b"\r\n\r\n")
        .map(|p| (p, p + 4))
        .ok_or(NormalizerError::MissingBody)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(s.len());
    let b = s
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|p| p + 1)
        .unwrap_or(a);
    &s[a..b]
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Indeksi i thonjëzës mbyllëse të një stringu JSON që fillon te `i`.
fn string_end(s: &[u8], mut i: usize) -> Option<usize> {
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Skanon tokenët e stringjeve JSON nga `from` dhe kthen çelësin e radhës
/// (string i ndjekur nga `:`) si interval + pozicioni pas `:`.
fn next_json_key(s: &[u8], from: usize) -> Option<((usize, usize), usize)> {
    let mut i = from;
    while i < s.len() {
        if s[i] == b'"' {
            let end = string_end(s, i + 1)?;
            let j = skip_ws(s, end + 1);
            if j < s.len() && s[j] == b':' {
                return Some(((i + 1, end), j + 1));
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    None
}

/// Gjen vlerën numerike të çelësit; refuzon dublikatat.
fn find_unique_json_u64(body: &[u8], key: &str) -> Result<u64, NormalizerError> {
    let mut found: Option<u64> = None;
    let mut i = 0;
    while let Some((k, after_colon)) = next_json_key(body, i) {
        i = after_colon;
        if &body[k.0..k.1] != key.as_bytes() {
            continue;
        }
        if found.is_some() {
            return Err(NormalizerError::DuplicateKey);
        }
        let mut p = skip_ws(body, after_colon);
        let quoted = p < body.len() && body[p] == b'"';
        if quoted {
            p += 1;
        }
        let start = p;
        let mut v: u64 = 0;
        while p < body.len() && body[p].is_ascii_digit() {
            v = v
                .checked_mul(10)
                .and_then(|x| x.checked_add((body[p] - b'0') as u64))
                .ok_or(NormalizerError::Overflow)?;
            p += 1;
        }
        if p == start {
            return Err(NormalizerError::InvalidNumber);
        }
        if p < body.len() && body[p] == b'.' {
            p += 1;
            let fs = p;
            while p < body.len() && body[p].is_ascii_digit() {
                p += 1;
            }
            if p == fs {
                return Err(NormalizerError::InvalidNumber);
            }
        }
        if p < body.len() && matches!(body[p], b'e' | b'E') {
            return Err(NormalizerError::InvalidNumber);
        }
        if quoted {
            if p >= body.len() || body[p] != b'"' {
                return Err(NormalizerError::InvalidNumber);
            }
            p += 1;
        }
        let q = skip_ws(body, p);
        if q < body.len() && !matches!(body[q], b',' | b'}' | b']') {
            return Err(NormalizerError::InvalidNumber);
        }
        found = Some(v);
        i = p;
    }
    found.ok_or(NormalizerError::KeyNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    const RESP: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: sid=abc123; HttpOnly\r\n\r\n{\"name\": \"Arben Krasniqi\", \"iban\":\"AL47212110090000000235698741\", \"balance\": 15200, \"currency\": \"EUR\", \"note\": \"x \\\"balance\\\": 999999\"}";

    #[test]
    fn extracts_and_balances() {
        let c = AlgebraicNormalizer::normalize_and_mask(RESP, "balance", 10_000).unwrap();
        assert_eq!(c.target_value, 15_200);
        assert!(c.is_greater_or_equal);
        assert_eq!(c.margin(), Some(5_200));
        let c2 = AlgebraicNormalizer::normalize_and_mask(RESP, "balance", 20_000).unwrap();
        assert!(!c2.is_greater_or_equal);
        assert_ne!(c.commitment_hash, c2.commitment_hash);
        assert_eq!(c.payload_digest, c2.payload_digest);
    }

    #[test]
    fn masking_is_length_preserving_and_streaming_digest_matches() {
        let mut buf: Vec<u8> = RESP.to_vec();
        let n = AlgebraicNormalizer::mask_sensitive_in_place(&mut buf).unwrap();
        assert_eq!(n, 3); // cookie, name, iban
        assert_eq!(buf.len(), RESP.len());
        let s = core::str::from_utf8(&buf).unwrap();
        assert!(!s.contains("Arben") && !s.contains("AL47") && !s.contains("abc123"));
        assert!(s.contains("\"balance\": 15200"));
        let d: [u8; 32] = Sha256::digest(&buf).into();
        assert_eq!(d, AlgebraicNormalizer::masked_digest(RESP).unwrap());
        // maskimi është idempotent
        let mut again = buf.clone();
        AlgebraicNormalizer::mask_sensitive_in_place(&mut again).unwrap();
        assert_eq!(again, buf);
    }

    #[test]
    fn rejects_ambiguous_and_malformed() {
        let dup = b"{\"balance\": 1, \"balance\": 2}";
        assert_eq!(
            AlgebraicNormalizer::normalize_and_mask(dup, "balance", 0),
            Err(NormalizerError::DuplicateKey)
        );
        for bad in [
            &b"{\"balance\": -5}"[..],
            b"{\"balance\": 1e9}",
            b"{\"balance\": abc}",
            b"{\"balance\": 12x}",
        ] {
            assert_eq!(
                AlgebraicNormalizer::normalize_and_mask(bad, "balance", 0),
                Err(NormalizerError::InvalidNumber)
            );
        }
        assert_eq!(
            AlgebraicNormalizer::normalize_and_mask(
                b"{\"balance\": 99999999999999999999}",
                "balance",
                0
            ),
            Err(NormalizerError::Overflow)
        );
        assert_eq!(
            AlgebraicNormalizer::normalize_and_mask(b"{\"x\": 1}", "balance", 0),
            Err(NormalizerError::KeyNotFound)
        );
        assert_eq!(
            AlgebraicNormalizer::normalize_and_mask(b"HTTP/1.1 200 OK\r\n", "balance", 0),
            Err(NormalizerError::MissingBody)
        );
        assert_eq!(
            AlgebraicNormalizer::normalize_and_mask(b"{}", "bal\"ance", 0),
            Err(NormalizerError::InvalidKey)
        );
    }

    #[test]
    fn decimals_and_quoted() {
        let c = AlgebraicNormalizer::normalize_and_mask(
            b"{\"balance\": \"15200.99\"}",
            "balance",
            15_200,
        )
        .unwrap();
        assert_eq!(c.target_value, 15_200);
        assert!(c.is_greater_or_equal);
    }
}
