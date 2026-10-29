#!/usr/bin/env bash
# Validator benchmark (acceptance A-10): builds an open 8-shard market with
# funded traders in LiteSVM, loads every account into a fresh localhost
# test validator, then sends the same independent swaps over 1, 2, 4 and 8
# shards and records how many land per slot.
# Throwaway keys under a temp dir only; no user wallet or config is used.
# Requires: target/deploy/sharded_amm.so (scripts/build-linux.sh) and
# solana-test-validator on PATH.
# Usage: scripts/bench-validator.sh [rounds=16] [repeats=3] [out=target/bench/validator-bench.md]
set -euo pipefail
cd "$(dirname "$0")/.."
rounds=${1:-16}
repeats=${2:-3}
out=${3:-target/bench/validator-bench.md}
program=target/deploy/sharded_amm.so
[[ -f $program ]] || { echo "Missing $program; run scripts/build-linux.sh first." >&2; exit 1; }
mkdir -p "$(dirname "$out")"
out=$(realpath "$out")
run_dir=$(mktemp -d)
validator_pid=''
cleanup() {
  [[ -n $validator_pid ]] && { kill "$validator_pid" 2>/dev/null || true; wait "$validator_pid" 2>/dev/null || true; }
}
trap cleanup EXIT

program_id=$(cd tests/svm && cargo run -q --release --example bench_setup -- "$run_dir/bench" | tee /dev/stderr >/dev/null; python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["program_id"])' "$run_dir/bench/bench.json")
solana-test-validator --ledger "$run_dir/ledger" --bind-address 127.0.0.1 \
  --rpc-port 18899 --faucet-port 18902 --gossip-port 18903 --dynamic-port-range 19000-19100 \
  --bpf-program "$program_id" "$program" --account-dir "$run_dir/bench/accounts" \
  --log >"$run_dir/validator.log" 2>&1 &
validator_pid=$!
for ((attempt = 0; attempt < 240; attempt++)); do
  kill -0 "$validator_pid" 2>/dev/null || { tail -50 "$run_dir/validator.log" >&2; exit 1; }
  if curl --silent --fail --max-time 2 -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' http://127.0.0.1:18899 | grep -q '"ok"'; then
    {
      echo "<!-- solana-test-validator $(solana-test-validator --version | head -1), $(nproc) CPU, rounds=$rounds repeats=$repeats -->"
    } >"$out.meta"
    (cd tests/svm && cargo run -q --release --example bench_run -- "$run_dir/bench" http://127.0.0.1:18899 "$rounds" "$repeats" "$out")
    cat "$out.meta" "$out" >"$out.tmp" && mv "$out.tmp" "$out" && rm "$out.meta"
    echo "wrote $out"
    exit 0
  fi
  sleep 0.5
done
echo 'Validator health timeout' >&2
exit 1
