//! Noteri TLSNotary: merr pjesë në lidhjen TLS të përdoruesit me MPC dhe
//! nënshkruan atestimin (secp256k1). Nuk sheh të dhënat e lidhjes.

use anyhow::{anyhow, bail, Result};
use futures::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::compat::TokioAsyncReadCompatExt;

use tlsn::{
    attestation::{
        request::Request as AttestationRequest, signing::Secp256k1Signer, Attestation,
        AttestationConfig, CryptoProvider,
    },
    config::verifier::VerifierConfig,
    connection::{CertBinding, ConnectionInfo, TranscriptLength},
    transcript::ContentType,
    verifier::{VerifierCommitStart, VerifierOutput},
    webpki::RootCertStore,
    Session,
};

/// Çelësi publik (SEC1 i kompresuar, hex) i një çelësi privat secp256k1.
pub fn public_key_hex(signing_key: &[u8; 32]) -> Result<String> {
    let sk = k256::ecdsa::SigningKey::from_bytes(signing_key.into())?;
    Ok(hex::encode(
        sk.verifying_key().to_encoded_point(true).as_bytes(),
    ))
}

/// Ekzekuton anën e noterit për një sesion. `roots`: certifikatat rrënjë të
/// besuara për serverët (në prodhim `RootCertStore::mozilla()`).
pub async fn run<S>(socket: S, signing_key: &[u8; 32], roots: RootCertStore) -> Result<()>
where
    S: AsyncWrite + AsyncRead + Send + Sync + Unpin + 'static,
{
    let session = Session::new(socket.compat());
    let (driver, mut handle) = session.split();
    let driver_task = tokio::spawn(driver);

    let verifier_config = VerifierConfig::builder().root_store(roots).build()?;
    let verifier = match handle.new_verifier(verifier_config)?.commit().await? {
        VerifierCommitStart::Mpc(v) => v.accept().await?.run().await?,
        VerifierCommitStart::Proxy(v) => {
            v.reject(Some("pritet MPC-TLS")).await?;
            bail!("protokolli proxy nuk pranohet");
        }
    };
    let (
        VerifierOutput {
            transcript_commitments,
            ..
        },
        verifier,
    ) = verifier.verify().await?.accept().await?;
    let tls_transcript = verifier.tls_transcript().clone();
    verifier.close().await?;

    let app_len = |records: &[tlsn::transcript::Record]| -> usize {
        records
            .iter()
            .filter(|r| matches!(r.typ, ContentType::ApplicationData))
            .map(|r| r.ciphertext.len())
            .sum()
    };
    let sent_len = app_len(tls_transcript.sent());
    let recv_len = app_len(tls_transcript.recv());

    handle.close();
    let mut socket = driver_task.await??;

    // kërkesa e atestimit është e vogël (disa KB); kufiri pengon një klient që
    // dërgon pafundësisht bajte të mbushë memorien
    const MAX_REQUEST: u64 = 1 << 20;
    let mut request_bytes = Vec::new();
    (&mut socket)
        .take(MAX_REQUEST + 1)
        .read_to_end(&mut request_bytes)
        .await?;
    if request_bytes.len() as u64 > MAX_REQUEST {
        bail!("kërkesa e atestimit > 1 MiB");
    }
    let request: AttestationRequest = bincode::deserialize(&request_bytes)?;

    let mut provider = CryptoProvider::default();
    provider
        .signer
        .set_signer(Box::new(Secp256k1Signer::new(signing_key)?));
    let mut cfg = AttestationConfig::builder();
    cfg.supported_signature_algs(Vec::from_iter(provider.signer.supported_algs()));
    cfg.extension_validator(|extensions| {
        crate::wallet_from_extensions(extensions)
            .map(|_| ())
            .map_err(|error| tlsn::attestation::InvalidExtension::new(error.to_string()))
    });
    let cfg = cfg.build()?;

    let CertBinding::V1_2(binding) = tls_transcript.certificate_binding() else {
        return Err(anyhow!("version e pambështetur e lidhjes së certifikatës"));
    };
    let mut builder = Attestation::builder(&cfg).accept_request(request)?;
    builder
        .connection_info(ConnectionInfo {
            time: tls_transcript.time(),
            version: tls_transcript.version(),
            transcript_length: TranscriptLength {
                sent: sent_len as u32,
                received: recv_len as u32,
            },
        })
        .server_ephemeral_key(binding.server_ephemeral_key.clone())
        .transcript_commitments(transcript_commitments);
    let attestation = builder.build(&provider)?;

    socket.write_all(&bincode::serialize(&attestation)?).await?;
    socket.close().await?;
    Ok(())
}
