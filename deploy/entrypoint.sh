#!/usr/bin/env bash
# Rolet: prover | bridge | notary   (fly.toml → [processes])
set -euo pipefail
role="${1:-prover}"
case "$role" in
  prover)
    args=(serve --addr 0.0.0.0:8787 --backend rapidsnark --rapidsnark /app/bin/prover
          --witness-bin /app/circuit/main_c --wasm /app/circuit/main.wasm
          --zkey /app/keys/main_final.zkey --parallel "${PROVER_PARALLEL:-0}")
    if [ "${DEMO_MOCK_NOTARY:-0}" = "1" ]; then
      # Noteri i simuluar: kushdo mund të krijojë sesione të rreme. Vetëm devnet/demo,
      # dhe çelësi i tij nuk duhet shtuar kurrë në whitelist-ën e mainnet-it.
      args+=(--mock-notary-key /app/demo/mock_notary_key.pem --mock-notary-cert /app/demo/mock_notary_cert.pem)
    fi
    exec coprocessor "${args[@]}"
    ;;
  bridge)
    : "${BRIDGE_RSA_KEY_PEM:?vendosni sekretin BRIDGE_RSA_KEY_PEM (fly secrets set)}"
    umask 077
    key="$(mktemp)"; printf '%s\n' "$BRIDGE_RSA_KEY_PEM" > "$key"
    policy=/app/bridge-policy.json
    if [ -n "${BRIDGE_POLICY_JSON:-}" ]; then policy="$(mktemp)"; printf '%s' "$BRIDGE_POLICY_JSON" > "$policy"; fi
    exec zktls-bridge --addr 0.0.0.0:8790 --policy "$policy" --rsa-key "$key"
    ;;
  notary)
    : "${NOTARY_KEY:?vendosni sekretin NOTARY_KEY (32 bajte hex)}"
    exec zktls-notary --addr 0.0.0.0:7047
    ;;
  *)
    exec "$@"
    ;;
esac
