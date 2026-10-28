# Gjendja aktuale

Përditësuar: 28 tetor 2026. Repository: `github.com/intopic/1`.

**Vendimi i produktit:** nisje me **8 shard-e**, ID 0–7, maskë `0xff`; zgjerim me kapital të verifikuar deri në **56**, ID 0–55. Specifikimi i plotë është [docs/DEVELOPER_HANDOFF_8_SHARDS.md](docs/DEVELOPER_HANDOFF_8_SHARDS.md).

**Përmbledhje:** mjedisi Linux funksionon (ENG-00), specifikimi numerik dhe ADR-të janë shkruar (ENG-01), dhe maska + lifecycle i regjistrit janë implementuar si logjikë e testuar në host (ENG-02). **Ende nuk ka instruksione onchain, custody, ticks, LP ose swap.** Asnjë deklaratë për throughput, paralelizëm të matur, kapje MEV ose fitim nuk është provuar.

## Milestone-et

| ID | Statusi | Evidenca / çfarë mungon |
|---|---|---|
| ENG-00 Mjedisi | **PASS në Linux** | Agave 4.3.2 (SHA256 i verifikuar), host rustc 1.95.0, platform-tools v1.58. Build SBF për `sharded-amm`, `env-smoke`, `env-anchor`; IDL nga `anchor-lang` 1.2.1 `idl-build`; validator lokal ping/pong i konfirmuar. Komandat: `scripts/build-linux.sh`, `scripts/smoke-validator.sh`; CI në `.github/workflows/ci.yml`. Anchor CLI binar nuk u ekzekutua në këtë mjedis; IDL u gjenerua me të njëjtin mekanizëm që përdor CLI-ja. |
| ENG-01 ADR/numeric/provenance | **PROPOSED** | [numeric-spec.md](docs/numeric-spec.md), [ADR 0001–0004](docs/adr/README.md), [upstream-lock.json](docs/upstream-lock.json), [third-party-notices.md](docs/third-party-notices.md), vektorë reference [tick_vectors.json](docs/vectors/tick_vectors.json). Pret rishikim; vendimet e produktit në ADR-0004 janë të hapura. |
| ENG-02 Bitmap/lifecycle | **PASS si logjikë host** | `programs/sharded-amm/src/{constants,errors,math/bitmap,state/*}.rs`; 23 teste. Instruksionet që e përdorin janë ENG-03/04. |
| ENG-03 … ENG-10 | NOT IMPLEMENTED | — |

## Evidenca e ENG-00 (Linux, 2026-10-28)

Mjedisi: kontejner Linux x86_64, rustc 1.95.0 (`rust-toolchain.toml`), `solana-cli 4.3.2`, `cargo-build-sbf 4.4.0` me `--tools-version v1.58`.

| Artefakti | Bajt | SHA256 |
|---|---:|---|
| `target/deploy/sharded_amm.so` | 49 840 | `4e30c7f9ac9838ca914b00c4357becba696c8bfe9a7a0f9c8f8191d4233b2321` |
| `env-smoke/target/deploy/environment_smoke.so` | 10 608 | `5615bb8b68315cde7cef0436b5e0d673dcab16b640a69946e0b6fec726e19c9f` |
| `env-anchor/target/deploy/anchor_environment_smoke.so` | 50 080 | `87633337f8f018c5b4a78c72a95fb51e8e54f441ff4a599c8120bee430ea3923` |

Dy build-e radhazi në të njëjtin path dhanë hash-e identike. Hash-i i `environment_smoke.so` ndryshon nga ai i Windows-it (`581e0826…`) edhe pse madhësia është e njëjtë; build-et SBF përfshijnë path-e, prandaj hash-et krahasohen vetëm për të njëjtin path dhe toolchain. Raporti i plotë gjenerohet në `target/build-report.json`.

Validator-i lokal (`solana-test-validator`, `127.0.0.1:18899`) ngarkoi `env-smoke`; klienti konfirmoi `pong`, simulimi negativ dha `InvalidInstructionData`, 161 CU. Transaksion legacy, jo v1.

## Kufizime të njohura

- Programi `sharded-amm` ka `#[program]` bosh; IDL ka 0 instruksione. Program ID `B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy` është placeholder; keypair-i nuk është në repo.
- Kompjuteri Windows mbetet i bllokuar për WSL nga virtualizimi BIOS i fikur ([BIOS_VIRTUALIZATION.md](docs/BIOS_VIRTUALIZATION.md)). Kjo nuk bllokon më zhvillimin: build-et bëhen në Linux/CI.
- Modeli `crates/amm-model` mbetet CPMM kërkimor; nuk është reference CLMM.

## Hapi i radhës

ENG-03: `MarketConfig`, `ShardState`, PDA-të, `initialize_market` dhe `prepare_shard` me vault-e SPL Token, teste negative për çdo owner/mint/authority/seed/alias, dhe testim në SVM. Për këtë duhen vendimet e ADR-0004 për token program, rolet dhe `tick_spacing`; propozimet aktuale përdoren si default i dokumentuar derisa pronari t'i ndryshojë.

## Si riprodhohet

```bash
# Linux x86_64 me rustup, python3, node dhe Agave 4.3.2 në PATH
bash scripts/build-linux.sh
bash scripts/smoke-validator.sh
```

Në Windows, kontrollet e vjetra (`scripts/preflight.ps1`, `scripts/check.ps1`) mbeten për modelin host.
