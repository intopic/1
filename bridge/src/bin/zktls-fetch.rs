//! Ana e përdoruesit (CLI): lidhet me bankën përmes noterit dhe shkruan
//! prezantimin; me `--bridge` e dërgon edhe te ura dhe ruan `session.tls.json`.
//!
//! ```text
//! zktls-fetch --notary 127.0.0.1:7047 --url https://api.banka.al/v1/me \
//!     --header "Authorization: Bearer ..." --wallet <PUBKEY> \
//!     --bridge https://<app>.fly.dev:8790 --out session.tls.json
//! ```
//!
//! `--bridge` pranon `https://host[:port]`, ose `http://host:port` / `host:port` vetëm për localhost.
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use base64::Engine;
use clap::Parser;
use http_body_util::{BodyExt, Full};
use hyper::{body::Bytes, Request};
use hyper_util::rt::TokioIo;
use tlsn::webpki::{CertificateDer, RootCertStore};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls;
use zktls_bridge::prover::{notarize, present, FetchRequest};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7047")]
    notary: String,
    /// https://host[:port]/path
    #[arg(long)]
    url: String,
    /// "Emri: vlera" (Authorization/Cookie nuk zbulohen në prezantim)
    #[arg(long)]
    header: Vec<String>,
    /// Adresa TCP e serverit, nëse ndryshon nga host-i i URL-së (teste)
    #[arg(long)]
    connect: Option<String>,
    #[arg(long)]
    wallet: String,
    /// Read-only API request method. POST requires a matching bridge source policy.
    #[arg(long, default_value = "GET", value_parser = ["GET", "POST"])]
    method: String,
    /// Exact request body from a local file; avoid secrets in command-line arguments.
    #[arg(long)]
    body_file: Option<std::path::PathBuf>,
    /// Ura: `https://host[:port]`, ose `host:port` (HTTP, vetëm localhost). Pa të, shkruhet vetëm prezantimi (base64).
    #[arg(long)]
    bridge: Option<String>,
    #[arg(long, default_value = "session.tls.json")]
    out: std::path::PathBuf,
    /// VETËM PËR TESTE: certifikatë rrënjë shtesë (DER)
    #[arg(long)]
    test_root_der: Option<std::path::PathBuf>,
}

fn parse_url(url: &str) -> Result<(String, u16, String)> {
    let rest = url
        .strip_prefix("https://")
        .context("URL duhet të fillojë me https://")?;
    let (hostport, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse()?),
        None => (hostport, 443),
    };
    Ok((
        host.into(),
        port,
        if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
    ))
}

async fn send<S>(io: S, req: Request<Full<Bytes>>) -> Result<(hyper::StatusCode, Bytes)>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io)).await?;
    tokio::spawn(conn);
    let resp = sender.send_request(req).await?;
    let status = resp.status();
    Ok((status, resp.into_body().collect().await?.to_bytes()))
}

/// POST JSON te ura, me TLS (`https://`, certifikatat Mozilla) ose HTTP i thjeshtë.
async fn post_json(bridge: &str, path: &str, body: String) -> Result<String> {
    let (tls, rest) = match (
        bridge.strip_prefix("https://"),
        bridge.strip_prefix("http://"),
    ) {
        (Some(r), _) => (true, r),
        (_, Some(r)) => (false, r),
        _ => (false, bridge),
    };
    let hostport = rest.trim_end_matches('/');
    let host = hostport
        .rsplit_once(':')
        .map_or(hostport, |(h, _)| h)
        .to_string();
    // HTTP i pakriptuar vetëm brenda makinës: prezantimi mban përgjigjen e bankës
    let loopback = host == "localhost"
        || host
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !tls && !loopback {
        bail!("ura `{bridge}` pa TLS: përdorni https:// (HTTP lejohet vetëm për localhost)");
    }
    let addr = if hostport.contains(':') {
        hostport.to_string()
    } else {
        format!("{hostport}:{}", if tls { 443 } else { 80 })
    };
    let req = Request::post(path)
        .header("host", &host)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))?;
    let tcp = tokio::net::TcpStream::connect(&addr).await?;
    let (status, body) = if tls {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let cfg = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(host.clone())?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(name, tcp)
            .await?;
        send(stream, req).await?
    } else {
        send(tcp, req).await?
    };
    let body = String::from_utf8_lossy(&body).into_owned();
    if !status.is_success() {
        bail!("ura refuzoi ({status}): {body}");
    }
    Ok(body)
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = Args::parse();
    let (host, port, path) = parse_url(&a.url)?;
    let headers = a
        .header
        .iter()
        .map(|h| {
            let (k, v) = h
                .split_once(':')
                .context("header-i duhet të jetë \"Emri: vlera\"")?;
            Ok((k.trim().to_string(), v.trim().to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let roots = match &a.test_root_der {
        Some(p) => RootCertStore {
            roots: vec![CertificateDer(std::fs::read(p)?)],
        },
        None => RootCertStore::mozilla(),
    };
    let notary = tokio::net::TcpStream::connect(&a.notary).await?;
    let server =
        tokio::net::TcpStream::connect(a.connect.clone().unwrap_or(format!("{host}:{port}")))
            .await?;
    eprintln!("MPC-TLS me noterin {} → {host}{path} …", a.notary);
    let req = FetchRequest {
        server_name: host,
        path,
        headers,
        wallet: a.wallet.clone(),
        method: a.method,
        body: a
            .body_file
            .map(std::fs::read)
            .transpose()?
            .unwrap_or_default(),
    };
    let (att, secrets) = notarize(notary, server, &req, roots).await?;
    let pres = present(&att, &secrets)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bincode::serialize(&pres)?);
    match &a.bridge {
        None => {
            std::fs::write(&a.out, &b64)?;
            eprintln!("prezantimi u shkrua në {}", a.out.display());
        }
        Some(addr) => {
            let body = serde_json::json!({ "presentation": b64, "wallet": a.wallet }).to_string();
            std::fs::write(&a.out, post_json(addr, "/attest", body).await?)?;
            eprintln!("sesioni i atestuar u shkrua në {}", a.out.display());
        }
    }
    Ok(())
}
