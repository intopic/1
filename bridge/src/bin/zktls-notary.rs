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
    /// Sesione MPC-TLS njëkohësisht (secili zë qindra MB RAM dhe CPU); të tjerat refuzohen
    #[arg(long, env = "NOTARY_MAX_SESSIONS", default_value_t = 4)]
    max_sessions: usize,
    /// Afati i një sesioni të plotë (sekonda): një klient që hap lidhjen dhe hesht
    /// nuk e mban vendin përgjithmonë
    #[arg(long, env = "NOTARY_SESSION_TIMEOUT_SECS", default_value_t = 180)]
    session_timeout_secs: u64,
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
    println!(
        "noteri po dëgjon në {} (≤ {} sesione, afati {} s)",
        args.addr, args.max_sessions, args.session_timeout_secs
    );
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(args.max_sessions.max(1)));
    let timeout = std::time::Duration::from_secs(args.session_timeout_secs);
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("accept: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        // pa vend të lirë: mbyll lidhjen menjëherë (klienti provon sërish), në vend që
        // të nisë një sesion MPC që mbingarkon memorien
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            eprintln!(
                "{peer}: noteri është i zënë ({} sesione)",
                args.max_sessions
            );
            drop(socket);
            continue;
        };
        let roots = roots.clone();
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(timeout, zktls_bridge::notary::run(socket, &key, roots))
                .await
            {
                Ok(Ok(())) => println!("{peer}: sesion i noterizuar"),
                Ok(Err(e)) => eprintln!("{peer}: {e:#}"),
                Err(_) => eprintln!("{peer}: sesioni kaloi afatin"),
            }
        });
    }
}
