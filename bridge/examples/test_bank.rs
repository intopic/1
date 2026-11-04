//! Bankë testuese HTTPS (certifikata e fixture-s së TLSNotary, domeni `test-server.io`)
//! për të provuar rrjedhën reale noter → urë pa një bankë të vërtetë.
//!
//! ```text
//! cargo run --example test_bank -- 127.0.0.1:4443 15200
//! ```
use std::sync::Arc;

use tlsn_server_fixture_certs::{SERVER_CERT_DER, SERVER_KEY_DER};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or("127.0.0.1:4443".into());
    let balance: u64 = args
        .next()
        .map(|b| b.parse())
        .transpose()?
        .unwrap_or(15_200);
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(
                SERVER_CERT_DER.to_vec(),
            )],
            rustls::pki_types::PrivateKeyDer::Pkcs8(SERVER_KEY_DER.to_vec().into()),
        )?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("banka testuese në https://test-server.io (TCP {addr}), bilanci {balance}");
    loop {
        let (sock, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let Ok(mut tls) = acceptor.accept(sock).await else {
                return;
            };
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match tls.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let body = format!(
                r#"{{"name":"Arta Hoxha","iban":"AL35202111090000000001234567","balance":{balance},"currency":"EUR"}}"#
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tls.write_all(resp.as_bytes()).await;
            let _ = tls.shutdown().await;
        });
    }
}
