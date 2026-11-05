#!/usr/bin/env bash
# Rolet: prover | bridge | notary   (fly.toml → [processes])
set -euo pipefail
role="${1:-prover}"
# skedarët që krijojmë (çelësa, politika, input-e prove) i lexon vetëm pronari
umask 077
# Çdo rol sheh vetëm sekretet e veta: nëse dikush e kompromenton një proces, nuk
# merr çelësat e roleve të tjera nga mjedisi. Rekomandimi për prodhim mbetet tre
# aplikacione Fly të ndara (docs/DEPLOY.md), secili me sekretet e veta.
drop() { for v in "$@"; do unset "$v"; done; }
case "$role" in
  prover)
    drop BRIDGE_SIGNING_SEED BRIDGE_TAG_KEY BRIDGE_POLICY_JSON NOTARY_KEY
    args=(serve --addr 0.0.0.0:8787 --backend rapidsnark --rapidsnark /app/bin/prover
          --witness-bin /app/circuit/main_c --wasm /app/circuit/main.wasm
          --zkey /app/keys/main_final.zkey --parallel "${PROVER_PARALLEL:-0}")
    if [ "${DEMO_MOCK_NOTARY:-0}" = "1" ]; then
      # Noteri i simuluar: kushdo mund të krijojë sesione të rreme. Vetëm devnet/demo,
      # dhe çelësi i tij nuk duhet shtuar kurrë në whitelist-ën e mainnet-it.
      args+=(--mock-notary-key /app/demo/mock_notary_seed.hex)
    fi
    if [ -n "${SOLANA_RPC_URL:-}" ] && [ -n "${VERIFIER_PROGRAM:-}" ]; then
      # refuzim i hershëm i nullifier-ave të zënë (pa shpenzuar CPU për provën)
      args+=(--rpc-url "$SOLANA_RPC_URL" --verifier-program "$VERIFIER_PROGRAM")
    fi
    exec coprocessor "${args[@]}"
    ;;
  bridge)
    drop NOTARY_KEY PROVER_API_KEYS
    : "${BRIDGE_SIGNING_SEED:?vendosni sekretin BRIDGE_SIGNING_SEED (fly secrets set)}"
    : "${BRIDGE_TAG_KEY:?vendosni sekretin e qendrueshem BRIDGE_TAG_KEY (32 bajte hex)}"
    key="$(mktemp)"; printf '%s\n' "$BRIDGE_SIGNING_SEED" > "$key"
    policy=/app/bridge-policy.json
    if [ -n "${BRIDGE_POLICY_JSON:-}" ]; then policy="$(mktemp)"; printf '%s' "$BRIDGE_POLICY_JSON" > "$policy"; fi
    # çelësi tani është në skedar 0600; nuk i duhet më mjedisit (BRIDGE_TAG_KEY lexohet nga ura)
    drop BRIDGE_SIGNING_SEED BRIDGE_POLICY_JSON
    exec zktls-bridge --addr 0.0.0.0:8790 --policy "$policy" --signing-key "$key"
    ;;
  notary)
    drop BRIDGE_SIGNING_SEED BRIDGE_TAG_KEY BRIDGE_POLICY_JSON PROVER_API_KEYS
    : "${NOTARY_KEY:?vendosni sekretin NOTARY_KEY (32 bajte hex)}"
    exec zktls-notary --addr 0.0.0.0:7047
    ;;
  *)
    exec "$@"
    ;;
esac
