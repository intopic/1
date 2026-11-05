//! E2E me TLSNotary real (MPC-TLS), pa rrjet të jashtëm:
//! bankë testuese TLS (certifikatat e fixture-s së TLSNotary) ↔ prover ↔ noter,
//! prezantim → ura → `NotarizedSession` → verifikim RSA + provë Groth16 (qarku dev)
//! → verifikim me ekuacionin e programit Solana.

use std::sync::Arc;

use base64::Engine;
use rand::SeedableRng;
use tlsn::webpki::{CertificateDer, RootCertStore};
use tlsn_server_fixture_certs::{CA_CERT_DER, SERVER_CERT_DER, SERVER_DOMAIN, SERVER_KEY_DER};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;

use zktls_bridge::bridge::{AttestRequest, Bridge, Policy, Source};
use zktls_bridge::notary;
use zktls_bridge::prover::{notarize, present, FetchRequest};
use zktls_prover::mock::{load_private_key_pem, MockNotary};
use zktls_prover::prover::DevGroth16Prover;
use zktls_prover::solana::{verify_reference, SolanaVerifyingKey};
use zktls_prover::witness::{app_id_from_label, ValueRange, ZkWebProofWitness};

const NOTARY_KEY: [u8; 32] = [7u8; 32];
const RSA_PEM: &str = include_str!("../../tests/fixtures/mock_notary_key.pem");
const WALLET: &str = "4wBqpZM9xaSheZzJSMawUKKwhdpChKbZ5eu5ky4Vigw";
const TOKEN: &str = "Bearer sekret-i-perdoruesit-123";

fn bank_body(balance: u64) -> String {
    format!(
        r#"{{"name":"Arta Hoxha","iban":"AL35202111090000000001234567","balance":{balance},"currency":"EUR"}}"#
    )
}

/// Bankë testuese: një përgjigje JSON, kërkon token-in në Authorization.
async fn bank<S>(socket: S, balance: u64)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(
                SERVER_CERT_DER.to_vec(),
            )],
            rustls::pki_types::PrivateKeyDer::Pkcs8(SERVER_KEY_DER.to_vec().into()),
        )
        .unwrap();
    let mut tls = tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
        .accept(socket)
        .await
        .unwrap();
    let mut req = Vec::new();
    let mut buf = [0u8; 1024];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = tls.read(&mut buf).await.unwrap();
        assert!(n > 0);
        req.extend_from_slice(&buf[..n]);
    }
    let req = String::from_utf8(req).unwrap();
    let ok = req.contains(&format!("authorization: {TOKEN}"))
        || req.contains(&format!("Authorization: {TOKEN}"));
    let body = if ok {
        bank_body(balance)
    } else {
        r#"{"error":"unauthorized"}"#.into()
    };
    let status = if ok { "200 OK" } else { "401 Unauthorized" };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tls.write_all(resp.as_bytes()).await.unwrap();
    tls.shutdown().await.unwrap();
}

fn roots() -> RootCertStore {
    RootCertStore {
        roots: vec![CertificateDer(CA_CERT_DER.to_vec())],
    }
}

fn bridge() -> Bridge {
    Bridge {
        policy: Policy {
            notary_keys: vec![notary::public_key_hex(&NOTARY_KEY).unwrap()],
            sources: vec![Source {
                server: SERVER_DOMAIN.into(),
                path_prefix: "/v1/accounts/".into(),
                target_key: "balance".into(),
                account_key: "iban".into(),
                currency: Some("EUR".into()),
                currency_key: "currency".into(),
                allow_query: false,
                target_pointer: Some("/balance".into()),
                account_pointer: Some("/iban".into()),
                decimals: 0,
                issuer: Some("test-bank".into()),
            }],
            max_age_secs: 600,
        },
        signer: MockNotary {
            key: load_private_key_pem(RSA_PEM).unwrap(),
            certificate_pem: None,
            tag_secret: None,
        },
        roots: roots(),
    }
}

/// Sesion i plotë MPC-TLS: kthen prezantimin (base64).
async fn session(balance: u64, path: &str) -> String {
    let (notary_io, prover_io) = tokio::io::duplex(1 << 23);
    let (bank_io, client_io) = tokio::io::duplex(1 << 16);
    let n = tokio::spawn(async move { notary::run(notary_io, &NOTARY_KEY, roots()).await });
    tokio::spawn(bank(bank_io, balance));
    let req = FetchRequest {
        server_name: SERVER_DOMAIN.into(),
        path: path.into(),
        headers: vec![("Authorization".into(), TOKEN.into())],
        wallet: WALLET.into(),
    };
    let (att, secrets) = notarize(prover_io, client_io, &req, roots()).await.unwrap();
    n.await.unwrap().unwrap();
    let pres = present(&att, &secrets).unwrap();
    base64::engine::general_purpose::STANDARD.encode(bincode::serialize(&pres).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tlsnotary_to_groth16() {
    let b64 = session(15_200, "/v1/accounts/me").await;

    // token-i nuk shfaqet në prezantim
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .unwrap();
    assert!(!raw.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()));

    let now = zktls_bridge::bridge::unix_now();
    let br = bridge();
    let s = br
        .attest(
            &AttestRequest {
                presentation: b64.clone(),
                wallet: WALLET.into(),
            },
            now,
        )
        .unwrap();
    assert_eq!(s.server_name, SERVER_DOMAIN);

    // i njëjti pipeline si noteri i simuluar: RSA + normalizim + provë
    let vs = s.verify(10_000).unwrap();
    assert_eq!(vs.message.value, 15_200);
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let w = ZkWebProofWitness::from_session(
        &vs,
        ValueRange::at_least(10_000),
        app_id_from_label("tlsn"),
        None,
    )
    .unwrap();
    let prover = DevGroth16Prover::setup(&mut rng).unwrap();
    let (proof, inputs) = prover.prove(&w, &mut rng).unwrap();
    assert!(verify_reference(&SolanaVerifyingKey::from_ark(prover.vk()), &proof, &inputs).unwrap());

    // --- refuzimet ---
    // noter tjetër
    let mut other = bridge();
    other.policy.notary_keys = vec![notary::public_key_hex(&[9u8; 32]).unwrap()];
    let e = other.attest(
        &AttestRequest {
            presentation: b64.clone(),
            wallet: WALLET.into(),
        },
        now,
    );
    assert!(format!("{:#}", e.unwrap_err()).contains("noter i panjohur"));
    // domen tjetër në politikë
    let mut other = bridge();
    other.policy.sources[0].server = "api.banka-tjeter.al".into();
    assert!(other
        .attest(
            &AttestRequest {
                presentation: b64.clone(),
                wallet: WALLET.into()
            },
            now
        )
        .is_err());
    // ri-lidhje: i njëjti prezantim, portofol tjetër → refuzohet (portofoli është në sesionin TLS)
    let e = br.attest(
        &AttestRequest {
            presentation: b64.clone(),
            wallet: "11111111111111111111111111111111".into(),
        },
        now,
    );
    assert!(format!("{:#}", e.unwrap_err()).contains("nuk është ai i sesionit TLS"));
    // monedhë tjetër: 15 200 EUR nuk kalon si 15 200 ALL (dhe anasjelltas)
    let mut other = bridge();
    other.policy.sources[0].currency = Some("ALL".into());
    let e = other.attest(
        &AttestRequest {
            presentation: b64.clone(),
            wallet: WALLET.into(),
        },
        now,
    );
    assert!(format!("{:#}", e.unwrap_err()).contains("monedhë e gabuar"));
    // sesion i vjetër
    assert!(br
        .attest(
            &AttestRequest {
                presentation: b64.clone(),
                wallet: WALLET.into()
            },
            now + 3600
        )
        .is_err());
    // prezantim i ndryshuar (bajt i përmbysur në mes)
    let mut bad = raw.clone();
    let i = bad.len() / 2;
    bad[i] ^= 1;
    let bad = base64::engine::general_purpose::STANDARD.encode(bad);
    assert!(br
        .attest(
            &AttestRequest {
                presentation: bad,
                wallet: WALLET.into()
            },
            now
        )
        .is_err());

    // gjatësi gënjeshtare e transkriptit: refuzohet para çdo alokimi të madh
    let (sent, recv) = zktls_bridge::bridge::declared_transcript_sizes(&raw)
        .unwrap()
        .expect("prezantimi ka transkript");
    assert!(
        sent > 0 && sent < 4096 && recv > 0 && recv < 1 << 16,
        "{sent} {recv}"
    );
    let pat: Vec<u8> = [sent as u64, recv as u64]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let at: Vec<usize> = (0..raw.len() - 16)
        .filter(|&i| raw[i..i + 16] == pat[..])
        .collect();
    assert_eq!(at.len(), 1);
    let mut huge = raw.clone();
    huge[at[0] + 8..at[0] + 16].copy_from_slice(&(1u64 << 42).to_le_bytes());
    let e = br.attest(
        &AttestRequest {
            presentation: base64::engine::general_purpose::STANDARD.encode(huge),
            wallet: WALLET.into(),
        },
        now,
    );
    assert!(format!("{:#}", e.unwrap_err()).contains("transkripti deklaron"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_path_outside_policy() {
    let b64 = session(15_200, "/v1/loans/me").await;
    let e = bridge()
        .attest(
            &AttestRequest {
                presentation: b64,
                wallet: WALLET.into(),
            },
            zktls_bridge::bridge::unix_now(),
        )
        .unwrap_err();
    assert!(format!("{e:#}").contains("burim i palejuar"), "{e:#}");
}
