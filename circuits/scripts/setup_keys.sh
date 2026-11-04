#!/usr/bin/env bash
# Ceremonia e besimit Groth16: Perpetual Powers of Tau (PSE, faza 1) + faza 2 specifike për qarkun.
#
# KUJDES: faza 2 këtu ka vetëm një kontribut lokal + beacon. Për mainnet,
# organizoni një ceremoni me shumë pjesëmarrës (snarkjs zkey contribute nga palë
# të pavarura) dhe publikoni transkriptin.
set -euo pipefail
cd "$(dirname "$0")/.."

SNARKJS="${SNARKJS:-npx snarkjs}"

[ -f build/main.r1cs ] || bash scripts/compile.sh

# Fuqia e ptau zgjidhet nga numri i kufizimeve: 2^POWER >= kufizimet + hyrjet
if [ -z "${PTAU_POWER:-}" ]; then
  CONSTRAINTS="$($SNARKJS r1cs info build/main.r1cs | sed -n 's/.*# of Constraints: *\([0-9]*\).*/\1/p')"
  POWER=10
  while [ $((1 << POWER)) -lt $((CONSTRAINTS + 64)) ]; do POWER=$((POWER + 1)); done
  echo "Kufizime: $CONSTRAINTS -> ptau 2^$POWER"
else
  POWER="$PTAU_POWER"
fi
PTAU="build/ppot_0080_${POWER}.ptau"

if [ ! -f "$PTAU" ]; then
  echo "Shkarkimi i $PTAU ..."
  # Perpetual Powers of Tau (PSE), 80 kontribute. Bucket-i i vjetër i Hermez kthen 403.
  curl -fL --retry 3 -o "$PTAU" "https://pse-trusted-setup-ppot.s3.eu-central-1.amazonaws.com/pot28_0080/ppot_0080_${POWER}.ptau"
fi
# Verifikimi i plotë i ptau zgjat shumë (>30 min për 2^19); PPoT është verifikuar publikisht.
if [ "${VERIFY_PTAU:-0}" = "1" ]; then $SNARKJS powersoftau verify "$PTAU"; fi

time $SNARKJS groth16 setup build/main.r1cs "$PTAU" build/main_0000.zkey
ENTROPY="$(head -c 64 /dev/urandom | xxd -p -c 256)"
time $SNARKJS zkey contribute build/main_0000.zkey build/main_0001.zkey \
  --name="kontributi-lokal" -v -e="$ENTROPY"
BEACON="${BEACON:-$(head -c 32 /dev/urandom | xxd -p -c 256)}"
$SNARKJS zkey beacon build/main_0001.zkey build/main_final.zkey "$BEACON" 10 -n="beacon-final"
if [ "${VERIFY_ZKEY:-0}" = "1" ]; then $SNARKJS zkey verify build/main.r1cs "$PTAU" build/main_final.zkey; fi
$SNARKJS zkey export verificationkey build/main_final.zkey build/verification_key.json

echo "OK: build/main_final.zkey, build/verification_key.json"
echo "Konvertimi për Solana: cargo run -p coprocessor-cli -- export-vk --snarkjs-vk circuits/build/verification_key.json --out artifacts/vk.json"
