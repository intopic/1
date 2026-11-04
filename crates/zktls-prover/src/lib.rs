//! # zktls-prover
//!
//! Rrjedha e plotë off-chain:
//!
//! ```text
//! sesion i noterizuar (JSON) ──► tls_parser ──► VerifiedSession
//!        (HTTP + atestim RSA)        │  (RSA PKCS#1 v1.5 me triad-core MOD-06,
//!                                    │   normalizer MED-01, kontroll digest-i)
//!                                    ▼
//!                               witness ──► input.json për circuits/main.circom
//!                                    │
//!                                    ▼
//!                               prover ──► Groth16/BN254 ──► solana (bajte për programin)
//! ```
//!
//! Dy prapavija provimi:
//! * [`prover::SnarkjsProver`] – prodhim: qarku i plotë Circom (RSA-2048 + SHA-256
//!   + kushti) përmes `snarkjs groth16 fullprove`. Kërkon artefaktet e kompiluara.
//! * [`prover::DevGroth16Prover`] – zhvillim: qark arkworks vendas që provon
//!   kushtin `value ≥ threshold` dhe zotimin Poseidon, me të njëjtat 7 hyrje publike.
//!   RSA verifikohet natively (jashtë provës) – **jo i sigurt për prodhim**.

pub mod attestation;
pub mod error;
pub mod mock;
pub mod poseidon;
pub mod prover;
pub mod solana;
pub mod tls_parser;
pub mod witness;

pub use attestation::AttestationMessage;
pub use error::ProverError;
pub use tls_parser::{NotarizedSession, VerifiedSession};

/// Rendi i hyrjeve publike (i njëjtë për qarkun Circom, qarkun dev dhe programin Solana).
pub mod public_layout {
    pub const COMMITMENT: usize = 0;
    pub const ATTESTED_AT: usize = 1;
    pub const CLAIM_KEY_ID: usize = 2;
    pub const BANK_KEY_HASH: usize = 3;
    pub const MIN_THRESHOLD: usize = 4;
    pub const USER_PUBKEY_HI: usize = 5;
    pub const USER_PUBKEY_LO: usize = 6;
    pub const COUNT: usize = 7;
}
