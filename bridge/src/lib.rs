//! Noteri real TLSNotary dhe ura drejt atestimit RSA të qarkut zkTLS.
//!
//! Rrjedha:
//!
//! ```text
//! përdoruesi (prover)  ──MPC-TLS──  noteri (`notary::run`)      ──TLS──  banka
//!        │                               │ nënshkruan atestimin TLSNotary (secp256k1)
//!        │ prezantimi (Presentation)     ▼
//!        └──────────────────────────► ura (`bridge::Bridge::attest`)
//!                                        │ verifikon noterin, domenin, rrugën, freskinë
//!                                        ▼
//!                           NotarizedSession (mesazh 128 B + RSA) → prova Groth16
//! ```
//!
//! Noteri nuk e sheh kurrë fjalëkalimin/cookie-n: TLS-ja kryhet me MPC. Prezantimi
//! fsheh vlerat e header-ave sekretë (Authorization, Cookie); përgjigjja e bankës
//! zbulohet e plotë te ura, e cila e përdor vetëm për të nxjerrë vlerën dhe nuk e ruan.
//! Asgjë nga këto nuk del on-chain.

pub mod bridge;
pub mod notary;
pub mod prover;

/// Kufijtë e transkriptit që parapërpunohen në MPC-TLS (më i vogël = më i shpejtë).
pub const MAX_SENT_DATA: usize = 1 << 12;
pub const MAX_RECV_DATA: usize = 1 << 14;

/// Header-at që nuk zbulohen kurrë në prezantim.
pub const SECRET_HEADERS: [&str; 3] = ["authorization", "cookie", "x-api-key"];
