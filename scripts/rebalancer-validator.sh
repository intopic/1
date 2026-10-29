#!/usr/bin/env bash
# Rebalancer bot against a local validator (ADR-0006): builds the bench
# markets (bench_setup lists four rebalancer keys on the 8-shard one), loads
# them into a fresh localhost test validator, then runs the bot through a
# calm phase and a phase with concurrent trader swaps.
# Throwaway keys under a temp dir only; no user wallet or config is used.
# Requires: target/deploy/sharded_amm.so (scripts/build-linux.sh) and
# solana-test-validator on PATH.
# Usage: scripts/rebalancer-validator.sh [out=target/bench/rebalancer.md] [noise swaps=120]
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/bench/rebalancer.md}
noise=${2:-120}
[[ "$noise" =~ ^[0-9]+$ ]] || { echo 'noise must be an integer' >&2; exit 1; }
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

(cd tests/svm && cargo run -q --release --example bench_setup -- "$run_dir/bench" 64)
program_id=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["program_id"])' "$run_dir/bench/bench.json")
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
      commit=$(git rev-parse HEAD 2>/dev/null || echo unknown)
      dirty=$([[ -z $(git status --porcelain 2>/dev/null) ]] && echo clean || echo dirty)
      run=${GITHUB_RUN_ID:+, CI run ${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-}/actions/runs/$GITHUB_RUN_ID}
      echo "**Provenance:** commit \`$commit\` ($dirty), \`sharded_amm.so\` sha256 \`$(sha256sum "$program" | cut -d' ' -f1)\`, $(solana-test-validator --version | head -1), $(nproc) CPU$run."
      echo
    } >"$out.meta"
    (cd tests/svm && cargo run -q --release --example rebalancer -- "$run_dir/bench" http://127.0.0.1:18899 "$out" --noise "$noise")
    cat "$out.meta" "$out" >"$out.tmp" && mv "$out.tmp" "$out" && rm "$out.meta"
    echo "wrote $out"
    exit 0
  fi
  sleep 0.5
done
echo 'Validator health timeout' >&2
exit 1
