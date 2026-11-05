#!/usr/bin/env bash
# Deploy në Solana devnet: të dy programet + inicializimi me VK-në e qarkut Circom.
#
# Kërkon: solana CLI, anchor 0.31.1, node 22, portofol me ~6 SOL devnet
# (https://faucet.solana.com), dhe çelësat e qarkut te deploy/keys/
# (main_final.zkey + verification_key.json; shih docs/DEPLOY.md).
#
#   PROVER_URL=https://<app>.fly.dev bash scripts/deploy-devnet.sh
set -euo pipefail
cd "$(dirname "$0")/.."

RPC_URL="${RPC_URL:-https://api.devnet.solana.com}"
APP_LABEL="${APP_LABEL:-demo}"
KEYS="${KEYS:-deploy/keys}"
WALLET="${ANCHOR_WALLET:-$HOME/.config/solana/id.json}"

[ -f "$KEYS/verification_key.json" ] || { echo "mungon $KEYS/verification_key.json"; exit 1; }
[ -f "$WALLET" ] || { echo "mungon portofoli $WALLET (solana-keygen new)"; exit 1; }
echo "portofoli: $(solana address -k "$WALLET")  bilanci: $(solana balance -u "$RPC_URL" -k "$WALLET")"

# 1. ID-të e programeve nga çelësat në target/deploy (krijohen herën e parë)
export CARGO_TARGET_DIR="$PWD/target"
anchor keys sync
anchor build

# 2. deploy (program i përditësueshëm; portofoli = upgrade authority)
anchor deploy --provider.cluster "$RPC_URL" --provider.wallet "$WALLET"

# 3. VK e Circom në formatin e programit + inicializimi (app_id, whitelist, pool-i)
mkdir -p artifacts/devnet
cargo run -q --release -p coprocessor-cli -- export-vk \
  --snarkjs-vk "$KEYS/verification_key.json" --out artifacts/devnet/vk.json
( cd app && npm ci --no-audit --no-fund )
# afati global: një sesion TLS më i vjetër se 1 orë nuk pranohet (ndryshohet me
# MAX_ATTESTATION_AGE_SECS=...; 0 = pa afat global)
RPC_URL="$RPC_URL" APP_LABEL="$APP_LABEL" VK_JSON=artifacts/devnet/vk.json \
  MAX_ATTESTATION_AGE_SECS="${MAX_ATTESTATION_AGE_SECS:-3600}" \
  ANCHOR_WALLET="$WALLET" PROVER_URL="${PROVER_URL:-}" node app/scripts/init-localnet.mjs

echo
echo "Gati në devnet. Faqja: cd app && npm run build  (app/.env.local u shkrua)"
