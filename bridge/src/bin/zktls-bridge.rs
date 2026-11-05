//! Ura HTTP: `POST /attest {presentation, wallet}` → `NotarizedSession` (JSON),
//! i njëjti format si `POST /mock-session` i daemon-it `coprocessor serve`.
//!
//! ```text
//! zktls-bridge --policy bridge-policy.json --signing-key bridge_seed.hex
//! ```
use std::convert::Infallible;

use anyhow::Result;
use clap::Parser;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{body::Bytes, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tlsn::webpki::{CertificateDer, RootCertStore};
use zktls_bridge::bridge::{unix_now, AttestRequest, Bridge, Policy};
use zktls_prover::{eddsa::SigningKey, mock::MockNotary};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:8790")]
    addr: String,
    #[arg(long)]
    policy: std::path::PathBuf,
    /// 32-byte BabyJubjub EdDSA signing seed, hex file.
    #[arg(long)]
    signing_key: std::path::PathBuf,
    /// Stable secret (32-byte hex) for account_tag, independent of the signing seed.
    #[arg(long, env = "BRIDGE_TAG_KEY", hide_env_values = true)]
    tag_key: String,
    /// VETËM PËR TESTE: certifikatë rrënjë shtesë (DER)
    #[arg(long)]
    test_root_der: Option<std::path::PathBuf>,
}

fn reply(code: StatusCode, body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(code)
        .header("content-type", "application/json")
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-methods", "GET, POST, OPTIONS")
        .header("access-control-allow-headers", "content-type")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

async fn handle(
    bridge: std::sync::Arc<Bridge>,
    req: Request<hyper::body::Incoming>,
) -> Response<Full<Bytes>> {
    match (req.method(), req.uri().path()) {
        (&Method::OPTIONS, _) => reply(StatusCode::NO_CONTENT, String::new()),
        (&Method::GET, "/health") => reply(StatusCode::OK, r#"{"status":"ok"}"#.into()),
        (&Method::POST, "/attest") => {
            let body = match Limited::new(req.into_body(), 1 << 20).collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => {
                    return reply(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        r#"{"error":"> 1 MiB"}"#.into(),
                    )
                }
            };
            // verifikimi i prezantimit është punë CPU: jashtë thread-eve të I/O-së
            let res = tokio::task::spawn_blocking(move || {
                serde_json::from_slice::<AttestRequest>(&body)
                    .map_err(anyhow::Error::from)
                    .and_then(|r| bridge.attest(&r, unix_now()))
            })
            .await
            .unwrap_or_else(|e| Err(anyhow::anyhow!("gabim i brendshëm: {e}")));
            match res {
                Ok(s) => reply(StatusCode::OK, serde_json::to_string(&s).unwrap()),
                Err(e) => reply(
                    StatusCode::BAD_REQUEST,
                    serde_json::json!({ "error": format!("{e:#}") }).to_string(),
                ),
            }
        }
        _ => reply(StatusCode::NOT_FOUND, r#"{"error":"not found"}"#.into()),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = Args::parse();
    let policy: Policy = serde_json::from_slice(&std::fs::read(&a.policy)?)?;
    policy.validate()?;
    let tag_secret = Some(
        <[u8; 32]>::try_from(hex::decode(a.tag_key.trim())?.as_slice())
            .map_err(|_| anyhow::anyhow!("BRIDGE_TAG_KEY must be 32 bytes (64 hex)"))?,
    );
    let roots = match &a.test_root_der {
        Some(p) => RootCertStore {
            roots: vec![CertificateDer(std::fs::read(p)?)],
        },
        None => RootCertStore::mozilla(),
    };
    let bridge = std::sync::Arc::new(Bridge {
        policy,
        signer: MockNotary {
            key: SigningKey::from_hex(&std::fs::read_to_string(&a.signing_key)?)?,
            tag_secret,
        },
        roots,
    });
    let listener = tokio::net::TcpListener::bind(&a.addr).await?;
    println!("ura po dëgjon në http://{}", a.addr);
    loop {
        // një gabim i përkohshëm (p.sh. shumë skedarë të hapur) nuk e ndal urën
        let stream = match listener.accept().await {
            Ok((s, _)) => s,
            Err(e) => {
                eprintln!("accept: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let bridge = bridge.clone();
        tokio::spawn(async move {
            let svc = hyper::service::service_fn(move |req| {
                let bridge = bridge.clone();
                async move { Ok::<_, Infallible>(handle(bridge, req).await) }
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(std::time::Duration::from_secs(10))
                .serve_connection(TokioIo::new(stream), svc);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(60), conn).await;
        });
    }
}
