//! Ana e përdoruesit: lidhja HTTPS me bankën përmes MPC-TLS me noterin,
//! marrja e atestimit dhe ndërtimi i prezantimit për urën.

use std::future::IntoFuture;

use anyhow::{bail, Context, Result};
use futures::io::{AsyncReadExt as _, AsyncWriteExt as _};
use http_body_util::{BodyExt, Empty};
use hyper::{body::Bytes, Request};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

use tlsn::{
    attestation::{
        presentation::Presentation,
        request::{Request as AttestationRequest, RequestConfig},
        Attestation, CryptoProvider, Secrets,
    },
    config::{
        prove::ProveConfig, prover::ProverConfig, tls::TlsClientConfig,
        tls_commit::mpc::MpcTlsConfig,
    },
    connection::{HandshakeData, ServerName},
    prover::ProverOutput,
    transcript::TranscriptCommitConfig,
    webpki::RootCertStore,
    Session,
};
use tlsn_formats::http::{DefaultHttpCommitter, HttpCommit, HttpTranscript};

use crate::{MAX_RECV_DATA, MAX_SENT_DATA, SECRET_HEADERS, WALLET_HEADER};

/// Një kërkesë GET drejt API-së së bankës/regjistrit.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Domeni TLS (SNI), p.sh. `api.banka.al`.
    pub server_name: String,
    /// Rruga, p.sh. `/v1/accounts/me`.
    pub path: String,
    /// Header-a shtesë (p.sh. `Authorization`). Vlerat e [`SECRET_HEADERS`] fshihen në prezantim.
    pub headers: Vec<(String, String)>,
    /// Portofoli Solana (base58): dërgohet si [`WALLET_HEADER`] brenda sesionit TLS,
    /// që atestimi të mos mund të kërkohet për një portofol tjetër.
    pub wallet: String,
}

/// Kryen kërkesën HTTPS me noterin si palë MPC dhe kthen atestimin + sekretet.
///
/// * `notary_socket` – lidhja me noterin (TCP/WebSocket)
/// * `server_socket` – lidhja TCP me serverin e bankës
pub async fn notarize<N, S>(
    notary_socket: N,
    server_socket: S,
    req: &FetchRequest,
    roots: RootCertStore,
) -> Result<(Attestation, Secrets)>
where
    N: AsyncWrite + AsyncRead + Send + Sync + Unpin + 'static,
    S: AsyncWrite + AsyncRead + Send + Sync + Unpin + 'static,
{
    let session = Session::new(notary_socket.compat());
    let (driver, mut handle) = session.split();
    let driver_task = tokio::spawn(driver);

    let prover = handle
        .new_prover(ProverConfig::builder().build()?)?
        .commit(
            MpcTlsConfig::builder()
                .max_sent_data(MAX_SENT_DATA)
                .max_recv_data(MAX_RECV_DATA)
                .build()?,
        )
        .await?;

    let (tls_connection, prover) = prover.connect(
        TlsClientConfig::builder()
            .server_name(ServerName::Dns(req.server_name.clone().try_into()?))
            .root_store(roots)
            .build()?,
        server_socket.compat(),
    )?;
    let prover_task = tokio::spawn(prover.into_future());

    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(tls_connection.compat())).await?;
    tokio::spawn(connection);

    let mut builder = Request::builder()
        .uri(&req.path)
        .header("Host", &req.server_name)
        .header("Accept", "application/json")
        // TLSNotary nuk mbështet kompresimin
        .header("Accept-Encoding", "identity")
        .header("Connection", "close")
        .header(WALLET_HEADER, &req.wallet);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let response = sender
        .send_request(builder.body(Empty::<Bytes>::new())?)
        .await?;
    let status = response.status();
    // lexo trupin që transkripti të jetë i plotë
    let _ = response.into_body().collect().await?;
    if !status.is_success() {
        bail!("serveri ktheu {status}");
    }

    let mut prover = prover_task.await??;
    let transcript = HttpTranscript::parse(prover.transcript())?;

    let mut commit = TranscriptCommitConfig::builder(prover.transcript());
    DefaultHttpCommitter::default().commit_transcript(&mut commit, &transcript)?;
    let transcript_commit = commit.build()?;

    let mut rc = RequestConfig::builder();
    rc.transcript_commit(transcript_commit);
    let request_config = rc.build()?;

    let mut pc = ProveConfig::builder(prover.transcript());
    if let Some(c) = request_config.transcript_commit() {
        pc.transcript_commit(c.clone());
    }
    let ProverOutput {
        transcript_commitments,
        transcript_secrets,
        ..
    } = prover.prove(&pc.build()?).await?;

    let prover_transcript = prover.transcript().clone();
    let tls_transcript = prover.tls_transcript().clone();
    prover.close().await?;

    let mut ab = AttestationRequest::builder(&request_config);
    ab.server_name(ServerName::Dns(req.server_name.clone().try_into()?))
        .handshake_data(HandshakeData {
            certs: tls_transcript
                .server_cert_chain()
                .context("mungon zinxhiri i certifikatave")?
                .to_vec(),
            sig: tls_transcript
                .server_signature()
                .context("mungon nënshkrimi i serverit")?
                .clone(),
            binding: tls_transcript.certificate_binding().clone(),
        })
        .transcript(prover_transcript)
        .transcript_commitments(transcript_secrets, transcript_commitments);
    let provider = CryptoProvider::default();
    let (request, secrets) = ab.build(&provider)?;

    handle.close();
    let mut socket = driver_task.await??;
    socket.write_all(&bincode::serialize(&request)?).await?;
    socket.close().await?;
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await?;
    let attestation: Attestation = bincode::deserialize(&bytes)?;
    request.validate(&attestation, &provider)?;
    Ok((attestation, secrets))
}

/// Ndërton prezantimin për urën: zbulon rreshtin e kërkesës, header-at jo-sekretë
/// dhe të gjithë përgjigjen; fsheh vlerat e [`SECRET_HEADERS`].
pub fn present(attestation: &Attestation, secrets: &Secrets) -> Result<Presentation> {
    let transcript = HttpTranscript::parse(secrets.transcript())?;
    let mut b = secrets.transcript_proof_builder();

    let request = &transcript.requests[0];
    b.reveal_sent(request.without_data())?;
    b.reveal_sent(&request.request.target)?;
    for h in &request.headers {
        let secret = SECRET_HEADERS
            .iter()
            .any(|s| h.name.as_str().eq_ignore_ascii_case(s));
        if secret {
            b.reveal_sent(h.without_value())?;
        } else {
            b.reveal_sent(h)?;
        }
    }
    b.reveal_recv(&transcript.responses[0])?;
    let proof = b.build()?;

    let provider = CryptoProvider::default();
    let mut pb = attestation.presentation_builder(&provider);
    pb.identity_proof(secrets.identity_proof())
        .transcript_proof(proof);
    Ok(pb.build()?)
}
