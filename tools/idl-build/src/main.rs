//! `idl-build <program-dir> <expected-name> <out.json>`
//!
//! Runs `anchor_lang_idl::build::IdlBuilder`, which is what `anchor idl build`
//! calls: it merges the program, event, error, constant and address blocks,
//! shortens module paths, sorts and verifies discriminators. The Anchor CLI
//! binary itself is not needed.

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, dir, name, out] = args.as_slice() else {
        eprintln!("usage: idl-build <program-dir> <expected-name> <out.json>");
        std::process::exit(2);
    };
    // anchor-lang-idl 0.1.4 passes the literal "+{toolchain}" to cargo when
    // RUSTUP_TOOLCHAIN is set (cargo run sets it). Without it, the nested
    // cargo picks the toolchain from the program's rust-toolchain.toml.
    std::env::remove_var("RUSTUP_TOOLCHAIN");
    let dir = std::fs::canonicalize(PathBuf::from(dir)).expect("program dir");
    let idl = anchor_lang_idl::build::IdlBuilder::new()
        .program_path(dir)
        .build()
        .unwrap_or_else(|e| {
            eprintln!("IDL build failed: {e}");
            std::process::exit(1);
        });
    if idl.metadata.name != *name {
        eprintln!("unexpected IDL name {}", idl.metadata.name);
        std::process::exit(1);
    }
    let json = serde_json::to_string_pretty(&idl).expect("serialize IDL");
    if let Some(parent) = PathBuf::from(out).parent() {
        std::fs::create_dir_all(parent).expect("create output dir");
    }
    std::fs::write(out, json + "\n").expect("write IDL");
    println!(
        "IDL {out}: address {}, {} instructions, {} accounts, {} events, {} errors, {} types",
        idl.address,
        idl.instructions.len(),
        idl.accounts.len(),
        idl.events.len(),
        idl.errors.len(),
        idl.types.len()
    );
}
