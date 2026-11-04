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
# CPP_WITNESS=1: gjeneron edhe dëshmitarin C++ (më i shpejtë se WASM; kërkon
# g++, make, nasm, libgmp-dev, nlohmann-json3-dev)
EXTRA=()
[ "${CPP_WITNESS:-0}" = "1" ] && EXTRA=(--c)
circom main.circom --r1cs --wasm --sym "${EXTRA[@]}" -l node_modules -o build
npx snarkjs r1cs info build/main.r1cs
if [ "${CPP_WITNESS:-0}" = "1" ]; then
  make -C build/main_cpp -j"$(nproc)"
  echo "OK: build/main_cpp/main (dëshmitari C++)"
fi
echo "OK: build/main.r1cs, build/main_js/main.wasm"
