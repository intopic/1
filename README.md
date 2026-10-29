# Sharded AMM — hapësira e zhvillimit

Produkti është projektuar si smart contract AMM/likuidimi në Solana që **nis me 8 shard-e aktive** dhe lejon zgjerim dinamik deri në **56**. Regjistri përdor maskë `u64`: nisja `0xff`, bitet e lejuara `0..55`. Shard-et e reja aktivizohen pas përgatitjes, financimit dhe kontrolleve të likuiditetit; keeper-i është shërbim ndihmës. **Gjendja (29 tetor 2026):** programi Anchor `programs/sharded-amm` është implementuar dhe testuar në CI: treg, bitmask 8→56, shard-e me vault-e të veta, CLMM me tick crossing, pozicione LP, tarifa LP/protokolli dhe swap exact-in. Testet ekzekutojnë programin real SBF në LiteSVM. Nuk është bërë deploy në devnet, nuk ka benchmark në validator dhe nuk ka motor huadhënieje. Një rishikim i jashtëm inxhinierik (29 tetor 2026, gjetjet F-01..F-06) u adresua; ky **nuk** është auditim sigurie. Detajet: [STATUS.md](STATUS.md).

**Dokumentet normative** (burimi i vetëm i së vërtetës kur dokumentet ndryshojnë): [numeric-spec.md](docs/numeric-spec.md), [account-layout.md](docs/account-layout.md) (ABI v1 dhe PDA), [tx-budget.md](docs/tx-budget.md), [ADR-të](docs/adr/README.md), [acceptance.md](docs/acceptance.md) dhe [STATUS.md](STATUS.md). Dokumentet e tjera janë **historike** (specifikime para implementimit); indeksi është në [docs/README.md](docs/README.md).

## Si fillojmë

Në Linux (ose CI), që është rruga e vetme e mbështetur:

```bash
bash scripts/build-linux.sh
bash scripts/smoke-validator.sh
```

Skriptet PowerShell më poshtë janë historike (mjedisi Windows i fazës ENG-00).


Nga PowerShell:

```powershell
Set-Location '<rrënja e repo-s>'
.\scripts\preflight.ps1
cargo test --workspace --offline
cargo fmt --all -- --check
```

Për kontrollin e plotë lokal (formatim, teste dhe Clippy):

```powershell
.\scripts\check.ps1
```

Preflight vetëm kontrollon mjedisin. Nuk instalon programe, nuk ndryshon konfigurimin Solana dhe nuk krijon wallet. Për të kontrolluar nëse janë të pranishme edhe komandat e fazës Solana:

```powershell
.\scripts\preflight.ps1 -RequireSolana
```

## Ku ndodhet puna

| Skedari / folderi | Qëllimi |
|---|---|
| `ROADMAP.md` | Faza, detyrat, varësitë dhe rezultati i pritshëm |
| `STATUS.md` | Çfarë është kryer, çfarë kaloi dhe hapi pasues |
| `docs/architecture.md` | Arkitektura dhe vendimet teknike |
| `docs/DEVELOPER_HANDOFF_8_SHARDS.md` | Dorëzimi i detajuar për developer-in: kërkesat, referencat Raydium/Orca, implementimi, siguria dhe kriteret e pranimit |
| `docs/OVERVIEW_RUST_ANCHOR_8_TO_56.md` | Dokumenti kryesor: nisja me 8, bitmask, PDA, instruksione, zgjerimi dhe struktura e propozuar Rust/Anchor |
| `docs/account-model.md` | PDA dhe llogaritë read/write për çdo veprim |
| `docs/security.md` | Rreziqet dhe kontrollet e sigurisë |
| `docs/acceptance.md` | Kushtet e kalimit në fazën pasuese |
| `crates/amm-model/` | Model reference Rust, me aritmetikë integer dhe teste |
| `scripts/preflight.ps1` | Inventari i mjeteve lokale |
| `toolchain.versions.json` | Versionet dhe SHA256 e mjeteve të zhvillimit |
| `docs/toolchain.md` | Build-i SBF, kufizimet native dhe rruga WSL |
| `env-smoke/` | Program i veçuar ping/pong për provën e mjedisit |
| `research/` | Kërkimi dhe eksperimenti fillestar Python |
| `programs/sharded-amm/` | Programi Anchor i implementuar: 18 instruksione, matematika CLMM, motori i swap-it dhe i likuiditetit |
| `tests/svm/` | Testet onchain me programin real SBF në LiteSVM (workspace më vete) |
| `crates/sim/` | Simulimi ENG-09 |
| `tools/idl-build/` | Gjeneron IDL-në e plotë me IdlBuilder të Anchor-it |
| `docs/numeric-spec.md`, `docs/adr/` | Specifikimi numerik dhe vendimet e arkitekturës (ENG-01) |
| `tools/reference/` | Referenca të pavarura (Python stdlib) për teste diferenciale |
| `scripts/build-linux.sh`, `scripts/smoke-validator.sh` | Build/test/IDL dhe prova e validator-it në Linux/CI (ENG-00) |

## Rregullat e projektit

- Nisja është me 8 shard-e aktive; 56 është kapaciteti maksimal dhe nuk financohet i tëri ditën e parë.
- Çdo shard ka likuiditetin dhe vault-et e veta; nuk kopjojmë të njëjtin kapital në disa shard-e.
- Nuk e quajmë një CLMM të vetëm me çmim global të menjëhershëm. Shard-et kanë çmime të pavarura.
- Asnjë pretendim TPS, fitimi ose “zero contention” pa benchmark të riprodhueshëm.
- Fillimisht mbështesim një çift tokenësh të thjeshtë dhe swap exact-in. Huadhënia, Token-2022 extensions dhe migrimi dinamik kalojnë porta të veçanta.
- Çelësat, wallet-et dhe sekretet nuk futen në kod ose dokumente.
- Kontrata e ardhshme duhet të ketë teste reference dhe negative përpara devnet; kapitali real kërkon verifikime dhe auditim të veçantë.

Kodi ruhet në `github.com/intopic/1`. `.tools`, `target/`, keypair-et dhe sekretet nuk publikohen (shih `.gitignore`).

Historikisht, build-i native në Windows u bllokua nga antivirusi dhe virtualizimi BIOS (`docs/BIOS_VIRTUALIZATION.md`). Prandaj i gjithë build-i dhe testimi bëhet në Linux/GitHub Actions.
