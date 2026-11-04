//! Ana e përdoruesit (CLI): lidhet me bankën përmes noterit dhe shkruan
//! prezantimin; me `--bridge` e dërgon edhe te ura dhe ruan `session.tls.json`.
//!
//! ```text
//! zktls-fetch --notary 127.0.0.1:7047 --url https://api.banka.al/v1/me \
//!     --header "Authorization: Bearer ..." --wallet <PUBKEY> \
//!     --bridge 127.0.0.1:8790 --out session.tls.json
//! ```
use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use base64::Engine;
use clap::Parser;
use tlsn::webpki::{CertificateDer, RootCertStore};
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
    /// Ura (host:port). Pa të, shkruhet vetëm prezantimi (base64).
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

/// POST minimal HTTP/1.1 (ura është lokale ose pas një reverse proxy).
fn post_json(addr: &str, path: &str, body: &str) -> Result<String> {
    let mut s = std::net::TcpStream::connect(addr)?;
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut resp = String::new();
    s.read_to_string(&mut resp)?;
    let (head, body) = resp
        .split_once("\r\n\r\n")
        .context("përgjigje e pavlefshme")?;
    if !head.starts_with("HTTP/1.1 200") {
        bail!("ura refuzoi: {body}");
    }
    Ok(body.to_string())
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
            std::fs::write(&a.out, post_json(addr, "/attest", &body)?)?;
            eprintln!("sesioni i atestuar u shkrua në {}", a.out.display());
        }
    }
    Ok(())
}
