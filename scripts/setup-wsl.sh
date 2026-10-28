#!/usr/bin/env bash
# Run explicitly with bash from the sharded_amm project root. No sudo is run here.
set -euo pipefail
[[ $(uname -s) == Linux && $(uname -m) == x86_64 ]] || { echo 'Requires Linux x86_64 (Ubuntu/WSL).' >&2; exit 1; }
[[ -f Cargo.toml && -f env-smoke/Cargo.toml && -f scripts/check-wsl.sh ]] || { echo 'Run from the sharded_amm project root.' >&2; exit 1; }
project_root=$(pwd -P)
missing=()
for command_name in curl sha256sum tar bzip2 xz cc pkg-config; do
  command -v "$command_name" >/dev/null || missing+=("$command_name")
done
if ((${#missing[@]})); then
  printf 'Missing prerequisites: %s\n' "${missing[*]}" >&2
  echo 'Run explicitly: sudo apt-get update && sudo apt-get install -y ca-certificates curl bzip2 xz-utils build-essential pkg-config libssl-dev libudev-dev libclang-dev protobuf-compiler' >&2
  exit 1
fi
linux_tools="$project_root/.tools/linux"
downloads="$linux_tools/downloads"
mkdir -p "$downloads"

download_verified() {
  local url=$1 destination=$2 expected=$3
  if [[ ! -f "$destination" ]]; then
    curl --proto '=https' --tlsv1.2 --fail --location --retry 3 "$url" -o "$destination.partial"
    printf '%s  %s\n' "$expected" "$destination.partial" | sha256sum --check --status
    mv "$destination.partial" "$destination"
  fi
  printf '%s  %s\n' "$expected" "$destination" | sha256sum --check --status || { echo "Checksum failed: $destination" >&2; exit 1; }
}

# Separate Linux Rust state; do not change shell startup files or global defaults.
export CARGO_HOME="$linux_tools/cargo"
export RUSTUP_HOME="$linux_tools/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
if [[ ! -x "$CARGO_HOME/bin/rustup" ]]; then
  rustup_url='https://static.rust-lang.org/rustup/archive/1.28.2/x86_64-unknown-linux-gnu/rustup-init'
  curl --proto '=https' --tlsv1.2 --fail --location "$rustup_url.sha256" -o "$downloads/rustup-init.sha256"
  rustup_digest=$(awk '{print $1}' "$downloads/rustup-init.sha256")
  [[ $rustup_digest =~ ^[a-f0-9]{64}$ ]] || { echo 'Invalid official rustup checksum.' >&2; exit 1; }
  download_verified "$rustup_url" "$downloads/rustup-init-1.28.2" "$rustup_digest"
  chmod +x "$downloads/rustup-init-1.28.2"
  "$downloads/rustup-init-1.28.2" -y --no-modify-path --profile minimal --default-toolchain none
fi
"$CARGO_HOME/bin/rustup" toolchain install 1.95.0 --profile minimal
export RUSTUP_TOOLCHAIN=1.95.0

# Official immutable release assets. SHA256 values verified against GitHub listings.
download_verified 'https://github.com/otter-sec/anchor/releases/download/v1.2.1/anchor-1.2.1-x86_64-unknown-linux-gnu' \
  "$downloads/anchor-1.2.1-linux" '5c899379cbd9a843fa4a730a37ba7ab61357571ffc535036a9afa08e5cd0b99b'
download_verified 'https://github.com/anza-xyz/agave/releases/download/v4.3.2/solana-release-x86_64-unknown-linux-gnu.tar.bz2' \
  "$downloads/agave-4.3.2-linux.tar.bz2" '5c9599d7fa397375df69d58e8d4f550ee45463dd6d1ecad9e5a1f053ce8e0933'
download_verified 'https://github.com/anza-xyz/platform-tools/releases/download/v1.58/platform-tools-linux-x86_64.tar.bz2' \
  "$downloads/platform-tools-1.58-linux.tar.bz2" '7b82eb420b5aebd1e5590db0ae25c523eca06724d4238ae02139690535636c37'

# Portable Linux Node avoids Ubuntu's older default and Windows PATH leakage.
# Primary pinned checksum: https://nodejs.org/en/blog/release/v24.13.0
node_archive='node-v24.13.0-linux-x64.tar.xz'
node_digest='e798599612f4bb71333a3397ab0d095fd62214e115aea45aa858a145fc72d67e'
curl --proto '=https' --tlsv1.2 --fail --location \
  'https://nodejs.org/dist/v24.13.0/SHASUMS256.txt' -o "$downloads/node-24.13.0-SHASUMS256.txt"
node_manifest_digest=$(awk -v filename="$node_archive" '$2 == filename {print $1}' "$downloads/node-24.13.0-SHASUMS256.txt")
[[ $node_manifest_digest == "$node_digest" ]] || { echo 'Official Node manifest does not match pinned checksum.' >&2; exit 1; }
download_verified "https://nodejs.org/dist/v24.13.0/$node_archive" "$downloads/$node_archive" "$node_digest"
mkdir -p "$linux_tools/node"
tar -xJf "$downloads/$node_archive" -C "$linux_tools/node"
export PATH="$linux_tools/node/node-v24.13.0-linux-x64/bin:$PATH"
node -e 'if(process.platform!=="linux" || process.versions.node!=="24.13.0") throw Error("Unexpected portable Node binary")'
mkdir -p "$linux_tools/anchor/1.2.1" "$linux_tools/agave/4.3.2" "$linux_tools/platform-tools/v1.58"
install -m 755 "$downloads/anchor-1.2.1-linux" "$linux_tools/anchor/1.2.1/anchor"
tar -xjf "$downloads/agave-4.3.2-linux.tar.bz2" -C "$linux_tools/agave/4.3.2"
tar -xjf "$downloads/platform-tools-1.58-linux.tar.bz2" -C "$linux_tools/platform-tools/v1.58"
[[ -x "$linux_tools/platform-tools/v1.58/rust/bin/rustc" ]] || { echo 'Unexpected platform-tools archive layout.' >&2; exit 1; }

# The bundled build driver uses ~/.cache/solana, not necessarily XDG_CACHE_HOME.
# Link only an unused exact-version slot; never replace existing user tools.
platform_cache="$HOME/.cache/solana/v1.58/platform-tools"
mkdir -p "$(dirname "$platform_cache")"
if [[ -e "$platform_cache" || -L "$platform_cache" ]]; then
  [[ $(readlink -f "$platform_cache") == "$linux_tools/platform-tools/v1.58" ]] || {
    echo "Existing cache slot is preserved: $platform_cache. Review it before choosing this isolated toolchain." >&2
    exit 1
  }
else
  ln -s "$linux_tools/platform-tools/v1.58" "$platform_cache"
fi
"$CARGO_HOME/bin/rustup" toolchain link 1.95.0-sbpf-solana-v1.58 "$linux_tools/platform-tools/v1.58/rust"
"$linux_tools/platform-tools/v1.58/rust/bin/rustc" --version
"$linux_tools/agave/4.3.2/solana-release/bin/solana" --version
"$linux_tools/anchor/1.2.1/anchor" --version
echo 'Linux candidate installed. Run bash scripts/check-wsl.sh for separate build/runtime evidence.'
