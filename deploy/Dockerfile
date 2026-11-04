# syntax=docker/dockerfile:1
# Imazhi i vetëm për fly.io: daemon-i i provës (rapidsnark + dëshmitar C++),
# ura TLSNotary → RSA dhe noteri TLSNotary. Roli zgjidhet në `entrypoint.sh`.
#
#   docker build -f deploy/Dockerfile -t zktls .
#
# Çelësi i provës (`main_final.zkey`) duhet të jetë ai që i përgjigjet VK-së on-chain:
# vendoseni te deploy/keys/ (p.sh. `gh release download circuit-keys-N -D deploy/keys`)
# ose jepni `--build-arg ZKEY_URL=...`.

ARG CIRCOM_VERSION=v2.1.9
ARG RAPIDSNARK_VERSION=v0.0.8

# ---- binarët Rust ----
FROM rust:1-bookworm AS rust
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY bridge bridge
COPY tests tests
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/bridge/target \
    cargo build --release --locked -p coprocessor-cli \
 && cargo build --release --locked --manifest-path bridge/Cargo.toml --bin zktls-bridge --bin zktls-notary \
 && mkdir -p /out \
 && cp target/release/coprocessor bridge/target/release/zktls-bridge bridge/target/release/zktls-notary /out/

# ---- qarku: wasm + dëshmitari C++ (i njëjti circom si në CI → i njëjti r1cs si zkey-ja) ----
FROM ubuntu:24.04 AS circuit
ARG CIRCOM_VERSION
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      ca-certificates curl nodejs npm build-essential nasm libgmp-dev nlohmann-json3-dev \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /usr/local/bin/circom \
      "https://github.com/iden3/circom/releases/download/${CIRCOM_VERSION}/circom-linux-amd64" \
 && chmod +x /usr/local/bin/circom
WORKDIR /src/circuits
COPY circuits/ ./
RUN npm install --no-audit --no-fund && CPP_WITNESS=1 bash scripts/compile.sh \
 && mkdir -p /out && cp build/main_js/main.wasm build/cpp/main_c_cpp/main_c build/cpp/main_c_cpp/main_c.dat /out/

# ---- rapidsnark (binari i release-it, varianti me asm x86_64) ----
# KUJDES: varianti `noasm` doli ~3.5 herë më i ngadaltë (5.6 s kundrejt 1.6 s për provë).
FROM ubuntu:24.04 AS rapidsnark
ARG RAPIDSNARK_VERSION
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends ca-certificates curl unzip \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; mkdir -p /tmp/rs /out; cd /tmp/rs; \
    url="$(curl -fsSL "https://api.github.com/repos/iden3/rapidsnark/releases/tags/${RAPIDSNARK_VERSION}" \
      | grep -o '"browser_download_url": *"[^"]*linux-x86_64[^"]*"' | grep -v noasm | head -1 \
      | sed 's/.*"\(http[^"]*\)"/\1/')"; \
    case "$url" in *noasm*|"") echo "nuk u gjet varianti asm"; exit 1;; esac; \
    echo "rapidsnark: $url"; curl -fsSL -o pkg "$url"; \
    (unzip -q pkg || tar xzf pkg); \
    cp "$(find . -type f -name prover | head -1)" /out/prover; chmod +x /out/prover

# ---- imazhi final ----
FROM ubuntu:24.04
ARG ZKEY_URL=""
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      ca-certificates curl libgmp10 libsodium23 libomp5 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 -m zk
WORKDIR /app
COPY --from=rust /out/ /app/bin/
COPY --from=rapidsnark /out/prover /app/bin/prover
COPY --from=circuit /out/ /app/circuit/
COPY deploy/keys/ /app/keys/
RUN set -eu; if [ ! -f /app/keys/main_final.zkey ] && [ -n "$ZKEY_URL" ]; then \
      curl -fL --retry 3 -o /app/keys/main_final.zkey "$ZKEY_URL"; fi; \
    test -s /app/keys/main_final.zkey || { echo "mungon deploy/keys/main_final.zkey (ose ZKEY_URL)"; exit 1; }
# VETËM PËR DEMO: çelësi i noterit të simuluar (publik në depo!) për /mock-session
COPY tests/fixtures/mock_notary_key.pem tests/fixtures/mock_notary_cert.pem /app/demo/
COPY bridge/bridge-policy.example.json /app/bridge-policy.json
COPY deploy/entrypoint.sh /app/entrypoint.sh
# kontroll: bibliotekat dinamike të binarëve C++ janë të pranishme
RUN chmod +x /app/entrypoint.sh \
 && for b in /app/bin/prover /app/circuit/main_c /app/bin/coprocessor; do \
      if ldd "$b" | grep -q "not found"; then ldd "$b"; exit 1; fi; done
USER zk
ENV PATH="/app/bin:${PATH}"
EXPOSE 8787 8790 7047
ENTRYPOINT ["/app/entrypoint.sh"]
CMD ["prover"]
