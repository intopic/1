# solana-zktls-coprocessor – skriptet e automatizimit
.PHONY: all build test bench lint fmt circuits setup-keys program-build program-test anchor-test demo clean

PROGRAM_MANIFEST := programs/solana-zk-verifier/Cargo.toml
CLI := cargo run -q --release -p coprocessor-cli --
DEMO := target/demo
# Portofol demo (çdo çelës publik base58 vlen; për on-chain përdorni `solana address`)
WALLET ?= 4wBqpZM9xaSheZzJSMawUKKwhdpChKbZ5eu5ky4Vigw

all: build test

build:
	cargo build --workspace --release

test: program-test
	cargo test --workspace

bench:
	cargo bench -p triad-core --bench montgomery_bench

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all
	cd programs/solana-zk-verifier && cargo fmt

# --- Qarqet ZK (kërkon circom >= 2.1.6 dhe Node.js) ---
circuits:
	bash circuits/scripts/compile.sh

setup-keys:
	bash circuits/scripts/setup_keys.sh
	$(CLI) export-vk --snarkjs-vk circuits/build/verification_key.json --out artifacts/vk.json

# --- Programi Solana ---
program-build:
	cargo build-sbf --manifest-path $(PROGRAM_MANIFEST)

program-test:
	cargo test --manifest-path $(PROGRAM_MANIFEST)

anchor-test:
	npm install
	anchor test

# --- Demo lokale e CLI-së (pa Solana) ---
demo:
	mkdir -p $(DEMO)
	$(CLI) mock-session --key tests/fixtures/mock_notary_key.pem --cert tests/fixtures/mock_notary_cert.pem \
		--balance 15200 --wallet $(WALLET) --out $(DEMO)/session.tls.json
	$(CLI) setup --out-dir $(DEMO)
	$(CLI) prove --input $(DEMO)/session.tls.json --threshold 10000 --pk $(DEMO)/dev_pk.bin \
		--witness-out $(DEMO)/input.json --out $(DEMO)/proof.json
	$(CLI) verify --proof $(DEMO)/proof.json --vk $(DEMO)/vk.json
	@echo "bank_key_hash: $$($(CLI) bank-key-hash --input $(DEMO)/session.tls.json)"

clean:
	cargo clean
	cd programs/solana-zk-verifier && cargo clean
	rm -rf circuits/build artifacts
