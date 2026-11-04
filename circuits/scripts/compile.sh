#!/usr/bin/env bash
# Kompilimi i qarkut kryesor me circom 2.1.x
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v circom >/dev/null 2>&1; then
  echo "circom mungon. Instaloni: https://docs.circom.io/getting-started/installation/ (>= 2.1.6)" >&2
  exit 1
fi
[ -d node_modules ] || npm install

mkdir -p build
circom main.circom --r1cs --wasm --sym -l node_modules -o build
npx snarkjs r1cs info build/main.r1cs
echo "OK: build/main.r1cs, build/main_js/main.wasm"
