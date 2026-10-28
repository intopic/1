#!/usr/bin/env bash
# Builds a smoke program and runs only a newly started localhost validator.
set -euo pipefail
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Requires Linux x86_64.' >&2; exit 1; }
[[ -f Cargo.toml && -f env-smoke/Cargo.toml && -f scripts/smoke-client.mjs ]] || { echo 'Run from the sharded_amm project root.' >&2; exit 1; }
project_root=$(pwd -P)
linux_tools="$project_root/.tools/linux"
export CARGO_HOME="$linux_tools/cargo"
export RUSTUP_HOME="$linux_tools/rustup"
export RUSTUP_TOOLCHAIN=1.95.0
export PATH="$linux_tools/node/node-v24.13.0-linux-x64/bin:$linux_tools/anchor/1.2.1:$linux_tools/agave/4.3.2/solana-release/bin:$CARGO_HOME/bin:$PATH"
for command_name in rustc cargo rustup solana anchor cargo-build-sbf solana-test-validator solana-keygen curl node; do
  command -v "$command_name" >/dev/null || { echo "Missing prerequisite: $command_name. Run setup or install Linux Node.js >=20 explicitly." >&2; exit 1; }
done
node -e 'if(process.platform!=="linux" || Number(process.versions.node.split(".")[0])<20) throw Error("Require Linux Node.js >=20; Windows node.exe is unsuitable")'
[[ $(rustc --version) == 'rustc 1.95.0 '* ]] || { echo 'Unexpected host Rust version.' >&2; exit 1; }
[[ $(anchor --version) == 'anchor-cli 1.2.1' ]] || { echo 'Unexpected Anchor version.' >&2; exit 1; }
[[ $(solana --version) == 'solana-cli 4.3.2 '* ]] || { echo 'Unexpected Solana version.' >&2; exit 1; }
rustc --version
anchor --version
solana --version
cargo-build-sbf --version
"$linux_tools/platform-tools/v1.58/rust/bin/rustc" --version
# Explicit platform version prevents the bundled driver's v1.57 default.
# Cargo dependencies may be fetched; platform-tools installation is disabled.
cargo-build-sbf --manifest-path env-smoke/Cargo.toml --tools-version v1.58 --skip-tools-install
program="$project_root/env-smoke/target/deploy/environment_smoke.so"
[[ -s "$program" ]] || { echo 'SBF artifact missing.' >&2; exit 1; }

# Fail if any required local test port is occupied; never use an existing RPC.
node --input-type=module -e '
import net from "node:net";
for (const port of [18899,18900,18902,18903]) {
  await new Promise((resolve,reject)=>{const s=net.createServer();s.once("error",reject);s.listen(port,"127.0.0.1",()=>s.close(resolve));});
}'
run_dir=$(mktemp -d "$linux_tools/smoke-run.XXXXXXXX")
validator_pid=''
cleanup() {
  if [[ -n "$validator_pid" ]]; then
    kill "$validator_pid" 2>/dev/null || true
    wait "$validator_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
solana-keygen new --no-bip39-passphrase --silent --outfile "$run_dir/program-keypair.json"
program_id=$(solana-keygen pubkey "$run_dir/program-keypair.json")
solana-test-validator --ledger "$run_dir/ledger" --bind-address 127.0.0.1 \
  --rpc-port 18899 --faucet-port 18902 --gossip-port 18903 --dynamic-port-range 19000-19100 \
  --log --bpf-program "$program_id" "$program" >"$run_dir/validator.log" 2>&1 &
validator_pid=$!
ready=false
for ((attempt=0; attempt<120; attempt++)); do
  kill -0 "$validator_pid" 2>/dev/null || { echo "Validator exited; inspect $run_dir/validator.log" >&2; exit 1; }
  if curl --silent --fail --max-time 2 -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' http://127.0.0.1:18899 | \
    node -e 'let s="";for await(const c of process.stdin)s+=c;if(JSON.parse(s).result!=="ok")process.exit(1)' --input-type=module; then
    ready=true
    break
  fi
  sleep 0.5
done
[[ $ready == true ]] || { echo "Validator health timeout; inspect $run_dir/validator.log" >&2; exit 1; }
node scripts/smoke-client.mjs "$program_id" >"$run_dir/report.json"
cat "$run_dir/report.json"
echo "Smoke evidence: $run_dir"
echo 'SBF/local runtime checked; this is not an Anchor build/IDL or transaction v1/AMM verification.'
