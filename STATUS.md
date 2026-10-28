# Gjendja aktuale

Përditësuar: 28 tetor 2026. Repository: `github.com/intopic/1`.

**Vendimi i produktit:** nisje me **8 shard-e**, ID 0–7, maskë `0xff`; zgjerim me kapital të verifikuar deri në **56**, ID 0–55. Specifikimi i plotë është [docs/DEVELOPER_HANDOFF_8_SHARDS.md](docs/DEVELOPER_HANDOFF_8_SHARDS.md).

**Përmbledhje:** mjedisi Linux/CI funksionon (ENG-00), specifikimi numerik dhe ADR-të janë shkruar (ENG-01), maska + lifecycle janë të testuara (ENG-02), krijimi i tregut e i shard-eve me vault-e është onchain me teste SVM (ENG-03, pjesërisht), dhe matematika CLMM është e testuar bit-për-bit kundrejt një reference të pavarur (ENG-05). Të gjitha testet ekzekutohen në GitHub Actions. **Ende nuk ka instruksione për ticks, LP ose swap.** Asnjë deklaratë për throughput, paralelizëm të matur, kapje MEV ose fitim nuk është provuar.

## Milestone-et

| ID | Statusi | Evidenca / çfarë mungon |
|---|---|---|
| ENG-00 Mjedisi | **PASS në Linux** | Agave 4.3.2 (SHA256 i verifikuar), host rustc 1.95.0, platform-tools v1.58. Build SBF për `sharded-amm`, `env-smoke`, `env-anchor`; IDL nga `anchor-lang` 1.2.1 `idl-build`; validator lokal ping/pong i konfirmuar. Komandat: `scripts/build-linux.sh`, `scripts/smoke-validator.sh`; CI në `.github/workflows/ci.yml`. Anchor CLI binar nuk u ekzekutua në këtë mjedis; IDL u gjenerua me të njëjtin mekanizëm që përdor CLI-ja. |
| ENG-01 ADR/numeric/provenance | **PROPOSED** | [numeric-spec.md](docs/numeric-spec.md), [ADR 0001–0004](docs/adr/README.md), [upstream-lock.json](docs/upstream-lock.json), [third-party-notices.md](docs/third-party-notices.md), vektorë reference [tick_vectors.json](docs/vectors/tick_vectors.json). Pret rishikim; vendimet e produktit në ADR-0004 janë të hapura. |
| ENG-02 Bitmap/lifecycle | **PASS si logjikë host** | `programs/sharded-amm/src/{constants,errors,math/bitmap,state/*}.rs`; 23 teste. Instruksionet që e përdorin janë ENG-03/04. |
| ENG-03 Accounts/PDA/custody | **PARTIAL** | `initialize_market`, `prepare_shard`, `set_paused`, `update_fees` me `MarketConfig`, `ShardRegistry`, `ShardState`, vault-e SPL për shard ([account-layout.md](docs/account-layout.md)). 18 teste SVM (LiteSVM 0.18.1, `tests/svm`) në CI: role, signer, seeds, mint/token program të gabuar, Token-2022, alias vault-esh, ID 56/63/255, generation ≠ 0, dyfishim, izolim byte-for-byte, madhësitë e llogarive; pauser pauzon dhe vetëm admin rifillon; pauza bllokon `prepare_shard`; tarifat vetëm nga admin, brenda kufijve, `tick_spacing` i pandryshuar. Mungojnë: timelock-u i admin-it dhe tërheqja/refund para nisjes (ENG-04). |
| ENG-05 Matematika CLMM | **PASS si logjikë host** | `programs/sharded-amm/src/math/{wide,tick_math,sqrt_price_math,swap_step,fee_math,liquidity_math}.rs`. U256/U512 (`uint` 0.10), tick ↔ sqrt price si floor ekzakt, delta-t me rrumbullakim në favor të pool-it, hapi i swap-it me `fee_nominal`/`rounding_dust` (N05), gabim në gap me L = 0, fee growth U256 Q128 checked (N07, me `inside` mod 2^256). Teste: 20 njësi/vetive + 6 grupe diferenciale bit-për-bit kundrejt [referencës Python](tools/reference/clmm_vectors.py) (400 ticks, 400 delta, 300 çmime, 600 hapa swap, 200 fee growth); CI kontrollon që tabela dhe vektorët rigjenerohen identikë. Mungojnë: thirrja nga instruksionet, matja e CU në SBF, provat formale P01–P05. |
| ENG-04, ENG-06 … ENG-10 | NOT IMPLEMENTED | Rendi pas [rishikimit të kërkimit](docs/research/2026-10-28-team-research-review.md): ENG-05b CPMM baseline → ENG-04 seed/cancel/refund → ENG-06 swap një-shard. |

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

- IDL ka 2 instruksione (`initialize_market`, `prepare_shard`); pa likuiditet, swap ose tarifa. Program ID `B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy` është placeholder; keypair-i nuk është në repo.
- Kompjuteri Windows mbetet i bllokuar për WSL nga virtualizimi BIOS i fikur ([BIOS_VIRTUALIZATION.md](docs/BIOS_VIRTUALIZATION.md)). Kjo nuk bllokon më zhvillimin: build-et bëhen në Linux/CI.
- Modeli `crates/amm-model` mbetet CPMM kërkimor; nuk është reference CLMM.

## Hapi i radhës

ENG-05b (CPMM baseline me ledger para/pas), pastaj ENG-04 (seed, finalize, cancel/refund) që përdor matematikën e ENG-05. CU e `tick_at_sqrt_price` (kërkim binar) matet në ENG-06 para se të vendoset për optimizim. Asnjë transaksion v1 nuk është ekzekutuar ende; SAMM është prior art, prandaj nuk pretendojmë risi për AMM me shards.

## Si riprodhohet

```bash
# Linux x86_64 me rustup, python3, node dhe Agave 4.3.2 në PATH
bash scripts/build-linux.sh
bash scripts/smoke-validator.sh
```

Në Windows, kontrollet e vjetra (`scripts/preflight.ps1`, `scripts/check.ps1`) mbeten për modelin host.
