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

step "host: fmt, clippy, tests"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

step "sbf: sharded-amm"
cargo build-sbf --manifest-path programs/sharded-amm/Cargo.toml --tools-version "$tools_version"

step "sbf: env-smoke"
cargo build-sbf --manifest-path env-smoke/Cargo.toml --tools-version "$tools_version"

step "sbf: env-anchor"
cargo build-sbf --manifest-path env-anchor/Cargo.toml --tools-version "$tools_version"

# Anchor's IDL builder, invoked the same way `anchor idl build` does, so the
# IDL comes from the anchor-lang 1.2.1 `idl-build` feature without the CLI.
idl_for() {
  local crate_dir=$1 name=$2 out=$3
  local raw
  raw=$(cd "$crate_dir" && ANCHOR_IDL_BUILD_PROGRAM_PATH="$(pwd -P)" \
    cargo test __anchor_private_print_idl --features idl-build -- --show-output --quiet)
  RAW="$raw" python3 - "$name" "$out" <<'PY'
import json, os, re, sys
raw = os.environ["RAW"]
def block(tag):
    m = re.search(rf"--- IDL begin {tag} ---\n(.*?)\n--- IDL end {tag} ---", raw, re.S)
    if not m:
        sys.exit(f"IDL block '{tag}' not found")
    return m.group(1)
idl = json.loads(block("program"))
address = json.loads(json.loads(block("address")))
idl["address"] = address
if idl["metadata"]["name"] != sys.argv[1]:
    sys.exit(f"unexpected IDL name {idl['metadata']['name']}")
os.makedirs(os.path.dirname(sys.argv[2]), exist_ok=True)
with open(sys.argv[2], "w") as f:
    json.dump(idl, f, indent=2)
    f.write("\n")
print(f"IDL {sys.argv[2]}: address {address}, {len(idl['instructions'])} instructions")
PY
}

step "idl"
idl_for programs/sharded-amm sharded_amm "$root/target/idl/sharded_amm.json"
idl_for env-anchor/programs/anchor-environment-smoke anchor_environment_smoke "$root/env-anchor/target/idl/anchor_environment_smoke.json"

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
