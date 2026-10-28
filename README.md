# Sharded AMM — hapësira e zhvillimit

Produkti është projektuar si smart contract AMM/likuidimi në Solana që **nis me 8 shard-e aktive** dhe lejon zgjerim dinamik deri në **56**. Regjistri përdor maskë `u64`: nisja `0xff`, bitet e lejuara `0..55`. Shard-et e reja aktivizohen pas përgatitjes, financimit dhe kontrolleve të likuiditetit; keeper-i është shërbim ndihmës. Dokumenti kryesor është [overview Rust/Anchor 8→56](docs/OVERVIEW_RUST_ANCHOR_8_TO_56.md). Kontrata dhe bitmask-i ende nuk janë implementuar; modeli aktual dhe programet provuese dallohen nga struktura e propozuar.

**Gjendja:** kërkim dhe prototip lokal. Nuk ka ende program Solana të deploy-uar, motor huadhënieje ose benchmark blockchain. Modeli Rust nuk përfshin tick crossing, LP accounting, oracle ose transferime SPL.

## Si fillojmë

Në Linux (ose CI):

```bash
bash scripts/build-linux.sh
bash scripts/smoke-validator.sh
```


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
| `programs/sharded-amm/` | Programi Anchor: konstantet, maska dhe lifecycle i regjistrit (ENG-02); ende pa instruksione |
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

Build-i SBF i programit provues ka kaluar. Anchor native është bllokuar nga Windows antivirus dhe validator-i native nuk arriti RPC ready. Pas restart-it WSL funksionon, por Ubuntu nuk niset sepse virtualizimi BIOS është i fikur. Shih `docs/BIOS_VIRTUALIZATION.md` dhe `STATUS.md`; kjo nuk ndryshon statusin e AMM-së së paimplementuar. Projekti `env-anchor` është përgatitur për provën e mëvonshme Anchor/IDL.
