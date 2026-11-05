//! Noteri real TLSNotary dhe ura drejt atestimit EdDSA të qarkut zkTLS.
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
//!                           NotarizedSession (mesazh 128 B + EdDSA) → prova Groth16
//! ```
//!
//! Noteri nuk e sheh kurrë fjalëkalimin/cookie-n: TLS-ja kryhet me MPC. Prezantimi
//! fsheh vlerat e header-ave sekretë (Authorization, Cookie); përgjigjja e bankës
//! zbulohet e plotë te ura, e cila e përdor vetëm për të nxjerrë vlerën dhe nuk e ruan.
//! Asgjë nga këto nuk del on-chain.

pub mod bridge;
pub mod notary;
pub mod prover;

/// Wallet binding covered by the notary's attestation signature, outside HTTP.
pub const WALLET_EXTENSION_ID: &[u8] = b"intopic.zktls.wallet/v1";

pub fn wallet_extension(wallet: &str) -> anyhow::Result<tlsn::attestation::Extension> {
    let value = bs58::decode(wallet).into_vec()?;
    anyhow::ensure!(value.len() == 32, "wallet must decode to 32 bytes");
    Ok(tlsn::attestation::Extension {
        id: WALLET_EXTENSION_ID.to_vec(),
        value,
    })
}

pub fn wallet_from_extensions(
    extensions: &[tlsn::attestation::Extension],
) -> anyhow::Result<String> {
    anyhow::ensure!(
        extensions.len() == 1,
        "exactly one signed wallet extension is required"
    );
    let extension = &extensions[0];
    anyhow::ensure!(
        extension.id == WALLET_EXTENSION_ID && extension.value.len() == 32,
        "invalid signed wallet extension"
    );
    Ok(bs58::encode(&extension.value).into_string())
}

/// Kufijtë e transkriptit që parapërpunohen në MPC-TLS (më i vogël = më i shpejtë).
pub const MAX_SENT_DATA: usize = 1 << 12;
pub const MAX_RECV_DATA: usize = 1 << 14;

/// Legacy header name remains reserved; new clients bind via a signed extension.
pub const WALLET_HEADER: &str = "x-zktls-wallet";

/// Header-at që nuk zbulohen kurrë në prezantim.
pub const SECRET_HEADERS: [&str; 3] = ["authorization", "cookie", "x-api-key"];
