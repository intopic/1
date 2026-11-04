//! Noteri TLSNotary (TCP). Një sesion MPC-TLS për çdo lidhje.
//!
//! ```text
//! NOTARY_KEY=<32 bajte hex> zktls-notary --addr 0.0.0.0:7047
//! ```
use anyhow::{Context, Result};
use clap::Parser;
use tlsn::webpki::{CertificateDer, RootCertStore};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7047")]
    addr: String,
    /// Çelësi privat secp256k1 i noterit (hex, 32 bajte)
    #[arg(long, env = "NOTARY_KEY", hide_env_values = true)]
    key: String,
    /// VETËM PËR TESTE: certifikatë rrënjë shtesë (DER) në vend të atyre Mozilla
    #[arg(long)]
    test_root_der: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let key: [u8; 32] = hex::decode(args.key.trim())?
        .try_into()
        .ok()
        .context("NOTARY_KEY duhet të ketë 32 bajte")?;
    let roots = match &args.test_root_der {
        Some(p) => RootCertStore {
            roots: vec![CertificateDer(std::fs::read(p)?)],
        },
        None => RootCertStore::mozilla(),
    };
    println!(
        "çelësi publik i noterit: {}",
        zktls_bridge::notary::public_key_hex(&key)?
    );
    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    println!("noteri po dëgjon në {}", args.addr);
    loop {
        let (socket, peer) = listener.accept().await?;
        let roots = roots.clone();
        tokio::spawn(async move {
            match zktls_bridge::notary::run(socket, &key, roots).await {
                Ok(()) => println!("{peer}: sesion i noterizuar"),
                Err(e) => eprintln!("{peer}: {e:#}"),
            }
        });
    }
}
