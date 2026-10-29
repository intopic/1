#!/usr/bin/env bash
# Reproducible Linux build for the workspace (ENG-00). Used by CI and locally.
#
# Prerequisites on PATH: rustup (rust-toolchain.toml pins the host compiler),
# Agave 4.3.2 (`cargo-build-sbf`), python3. Platform-tools v1.58 is selected
# explicitly. No wallet, keypair or cluster configuration is read or written.
#
# Output: target/idl/sharded_amm.json and target/build-report.json with the
# SHA256 of every built program. SBF bytes are path-sensitive, so hashes are
# only comparable between builds from the same checkout path and toolchain.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd -P)
tools_version=v1.58

step() { printf '\n== %s\n' "$*"; }

step "versions"
rustc --version
cargo --version
solana --version
cargo-build-sbf --version

step "reference: generated table and vectors are reproducible"
python3 -I tools/reference/tick_table.py | diff -u programs/sharded-amm/src/math/tick_table.rs -
python3 -I tools/reference/clmm_vectors.py | diff -q docs/vectors/clmm_vectors.json -

step "host: fmt, clippy, tests"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

step "reference: every tick of the domain is exact"
tick_table=$(mktemp)
python3 -I tools/reference/all_tick_prices.py "$tick_table"
AMM_FULL_TICK_TABLE="$tick_table" cargo test --release -p sharded-amm --test math_vectors full_tick_table -- --nocapture
rm -f "$tick_table"

step "sbf: sharded-amm"
cargo build-sbf --manifest-path programs/sharded-amm/Cargo.toml --tools-version "$tools_version"

step "idl (Anchor IdlBuilder: program, events, errors, address)"
# tools/idl-build calls anchor_lang_idl::build::IdlBuilder, the same code
# `anchor idl build` runs, so events and errors are merged into the IDL.
cargo build --release -p idl-build
idl_build="$root/target/release/idl-build"
"$idl_build" programs/sharded-amm sharded_amm "$root/target/idl/sharded_amm.json"
"$idl_build" env-anchor/programs/anchor-environment-smoke anchor_environment_smoke "$root/env-anchor/target/idl/anchor_environment_smoke.json"

step "svm: integration tests (LiteSVM, separate workspace), IDL acceptance included"
(cd tests/svm && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings && AMM_REQUIRE_IDL=1 cargo test -- --nocapture)

step "sbf: env-smoke"
cargo build-sbf --manifest-path env-smoke/Cargo.toml --tools-version "$tools_version"

step "sbf: env-anchor"
cargo build-sbf --manifest-path env-anchor/Cargo.toml --tools-version "$tools_version"

step "report"
python3 - "$root" <<'PY'
import hashlib, json, subprocess, sys, os
root = sys.argv[1]
def sha(p):
    return hashlib.sha256(open(p, "rb").read()).hexdigest()
def out(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip()
artifacts = [
    "target/deploy/sharded_amm.so",
    "target/idl/sharded_amm.json",
    "env-smoke/target/deploy/environment_smoke.so",
    "env-anchor/target/deploy/anchor_environment_smoke.so",
    "env-anchor/target/idl/anchor_environment_smoke.json",
]
report = {
    "git_revision": out("git", "-C", root, "rev-parse", "HEAD"),
    "git_dirty": bool(out("git", "-C", root, "status", "--porcelain")),
    "checkout_path": root,
    "rustc": out("rustc", "--version"),
    "solana": out("solana", "--version"),
    "cargo_build_sbf": out("cargo-build-sbf", "--version").splitlines()[0],
    "platform_tools": "v1.58",
    "artifacts": {a: {"bytes": os.path.getsize(os.path.join(root, a)), "sha256": sha(os.path.join(root, a))} for a in artifacts},
}
with open(os.path.join(root, "target/build-report.json"), "w") as f:
    json.dump(report, f, indent=2)
    f.write("\n")
print(json.dumps(report, indent=2))
PY
