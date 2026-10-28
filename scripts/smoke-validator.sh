#!/usr/bin/env bash
# Local runtime smoke (ENG-00): starts a fresh localhost test validator with
# env-smoke preloaded, runs scripts/smoke-client.mjs, then stops it.
# Uses an ephemeral program identity under a temp dir; no user wallet/config.
# Requires: a prior `cargo build-sbf` of env-smoke, solana-test-validator, node.
set -euo pipefail
cd "$(dirname "$0")/.."
program=env-smoke/target/deploy/environment_smoke.so
[[ -f $program ]] || { echo "Missing $program; run scripts/build-linux.sh first." >&2; exit 1; }
run_dir=$(mktemp -d)
validator_pid=''
cleanup() {
  [[ -n $validator_pid ]] && { kill "$validator_pid" 2>/dev/null || true; wait "$validator_pid" 2>/dev/null || true; }
}
trap cleanup EXIT
solana-keygen new --no-bip39-passphrase --silent --outfile "$run_dir/program-keypair.json"
program_id=$(solana-keygen pubkey "$run_dir/program-keypair.json")
solana-test-validator --ledger "$run_dir/ledger" --bind-address 127.0.0.1 \
  --rpc-port 18899 --faucet-port 18902 --gossip-port 18903 --dynamic-port-range 19000-19100 \
  --log --bpf-program "$program_id" "$program" >"$run_dir/validator.log" 2>&1 &
validator_pid=$!
for ((attempt = 0; attempt < 120; attempt++)); do
  kill -0 "$validator_pid" 2>/dev/null || { cat "$run_dir/validator.log" >&2; exit 1; }
  if curl --silent --fail --max-time 2 -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' http://127.0.0.1:18899 | grep -q '"ok"'; then
    node scripts/smoke-client.mjs "$program_id"
    exit 0
  fi
  sleep 0.5
done
echo 'Validator health timeout' >&2
exit 1
