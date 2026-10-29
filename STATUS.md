# Gjendja aktuale

Përditësuar: 29 tetor 2026. Repository: `github.com/intopic/1`, branch `claude/project-thread-j3s3pr`.

**Vendimi i produktit:** nisje me **8 shard-e**, ID 0–7, maskë `0xff`; zgjerim me kapital të verifikuar deri në **56**, ID 0–55. Specifikimi i plotë është [docs/DEVELOPER_HANDOFF_8_SHARDS.md](docs/DEVELOPER_HANDOFF_8_SHARDS.md).

**Përmbledhje:** programi onchain ka ciklin e plotë: krijim tregu, përgatitje shard-esh, çmim fillestar, tick arrays, pozicione LP, hapje atomike e 8 shard-eve, anulim me refund, swap exact-in, tarifa me timelock, tarifa protokolli dhe zgjerim deri në 56 shard-e. Gjithçka është testuar në CI: në host, në LiteSVM me programin real SBF, dhe në një workflow stresi paralel. **Nuk ka pasur auditim të jashtëm.** Nuk është bërë deploy në devnet: kjo kërkon një keypair të programit si sekret GitHub, të cilin e jep pronari. Asnjë deklaratë për throughput, paralelizëm të matur onchain, kapje MEV ose fitim nuk është provuar.

## Milestone-et

| ID | Statusi | Evidenca |
|---|---|---|
| ENG-00 Mjedisi | **PASS** | Agave 4.3.2 (SHA256 i verifikuar), rustc 1.95.0 (host) / 1.98.1 (SVM), platform-tools v1.58. `scripts/build-linux.sh`, `scripts/smoke-validator.sh`, `.github/workflows/ci.yml`. |
| ENG-01 ADR/numeric/provenance | **PROPOSED** | [numeric-spec.md](docs/numeric-spec.md), [ADR 0001–0004](docs/adr/README.md), [upstream-lock.json](docs/upstream-lock.json). Vendimet e hapura të produktit janë në ADR-0004 (freeze authority para mainnet). |
| ENG-02 Bitmap/lifecycle | **PASS** | Maska, statuset, `Cancelled`; teste njësi. |
| ENG-03 Accounts/PDA/custody | **PASS** | 18 teste SVM (13 llogari + 5 kontrolle): role, seeds, Token-2022 i refuzuar, alias vault-esh, ID ≥ 56, madhësitë e të gjitha llogarive ([account-layout.md](docs/account-layout.md)), tarifa me timelock 24 orë. |
| ENG-04 Seed/open/cancel | **PASS** | `eng04_lifecycle` (8 teste): hapje atomike e 8 shard-eve, readiness, llogari të gabuara, rregullat e çmimit, lidhja me shard/pronar, refund pas anulimit, pauza lejon vetëm daljet. |
| ENG-05 Matematika CLMM | **PASS** | Vektorë bit-për-bit kundër referencës Python. **Të 887 273 ticks** janë ekzakte (`all_tick_prices.py`, në CI). 7 teste vetish me 1M raste secili. `mul_div` me rrugë të shpejta, krahasuar me U512. |
| ENG-05b CPMM baseline | **PASS** | `crates/amm-model`: sulmi i inflacionit, ledger, sekuenca të rastit. |
| ENG-06 Swap | **PASS** | `eng06_swap` (6 teste). Punë e rastit onchain krahasohet bit-për-bit me motorin host: gjendja, tick arrays, bilancet dhe kodet e gabimit. Kontrollohen ruajtja e token-ave dhe dalja e plotë e të gjithëve. Testohen edhe slippage, dust, deadline, kufijtë e çmimit, llogari të gabuara, autoriteti i tarifave të protokollit dhe tarifat LP pro-rata. CU: ~42.5k një hap, ~9k për kalim, 10 kalime brenda 200k. |
| ENG-07 Zgjerimi 8 → 56 | **PASS** | `eng07_expansion` (3 teste): shard-et 8, 30 dhe 55 aktivizohen; rregullat e keeper-it, devijimi i çmimit, readiness, referenca, pauza; të 56 aktivë njëkohësisht. |
| ENG-08 WSOL/USDC | **PASS** | `eng08_wsol` (3 teste): vault-e native me `lamports = rent + amount`; wrap, swap dhe unwrap në një transaksion; lamports pa `SyncNative` nuk tregtohen. |
| ENG-09 Simulimi | **PASS (model)** | `crates/sim`: [rezultatet](docs/sim/eng09-results.md) dhe [analiza](docs/sim/eng09-analysis.md). Gjetja kryesore: me 8 shard-e, router-i duhet ta ndajë urdhrin nëpër shard-e, përndryshe kostoja e trader-it rritet disa herë. |
| ENG-10 Stres dhe sulme | **PASS** | `eng10_attacks` (5 skenarë sulmi) dhe [`stress.yml`](.github/workflows/stress.yml): 16 + 16 punë paralele. Rezultatet janë në seksionin më poshtë. |

## Stresi masiv (ENG-10)

Ekzekutimi i parë: [stress #1](https://github.com/intopic/1/actions/runs/40792480311), 29 tetor 2026, seed_base 1. Të 35 punët **kaluan pa asnjë dështim**. Puna u nda në 16 punë host dhe 16 punë on-chain që ekzekutohen paralelisht.

| Pjesa | Sasia në një ekzekutim | Çfarë kontrollohet |
|---|---|---|
| Vetitë matematikore | ~61 milionë raste | rrumbullakim ekzakt floor/ceil, çmimi tjetër që s'e kalon objektivin, hapat e swap-it në favor të pool-it, ndarja e input-it, fee growth, likuiditeti |
| Modeli i motorit | 3.2 milionë sekuenca me shumë LP, ~110 milionë tentativa swap (~11 milionë kalime ticks) | të gjitha invariantet pas çdo veprimi, gjendje e pandryshuar pas dështimit, dalje e plotë |
| CPMM baseline | 16 milionë sekuenca × 40 hapa | ledger, k, pa fitim nga round-trip |
| Ticks | 887 273 (i gjithë domain-i) | floor ekzakt dhe inversi |
| On-chain (programi SBF real në LiteSVM) | 16 × 20 000 veprime të rastit, dhe të gjitha suitat SVM 16 herë | çdo swap bit-për-bit me motorin host, ruajtja e token-ave, dalja e plotë |
| Simulimi ENG-09 | 32 seeds | përfundimet e [analizës](docs/sim/eng09-analysis.md) |

Ekzekutimi riprodhohet nga GitHub Actions → stress → Run workflow, me `scale` (shumëzues) dhe `seed_base`. Çdo punë e shtyp seed-in e vet.

## Kufizime të njohura

- **Pa auditim.** Testet zbulojnë gabime, por nuk provojnë mungesën e tyre.
- Program ID `B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy` është placeholder; keypair-i nuk është në repo. Deploy-i në devnet kërkon një sekret GitHub nga pronari.
- Swap-et që kalojnë më shumë se ~15 ticks të inicializuar kërkojnë `SetComputeUnitLimit`.
- Një swap i ndarë në 8 shard-e kërkon address lookup table për **madhësinë e paketës** (1 232 bajt). ALT **nuk** e rrit kufirin prej 64 llogarish: 8 shard-e × 3 tick arrays + ComputeBudget janë saktësisht 64, dhe çdo llogari shtesë (p.sh. System Program për wrap SOL) refuzohet. Llogaritja dhe testet reale v0 janë në [tx-budget.md](docs/tx-budget.md). Router-i/SDK (split routing) nuk është shkruar ende.
- Token-2022 refuzohet me qëllim në v1.
- `upgrade authority` dhe `freeze authority` janë vendime para mainnet (ADR-0004).

## Si riprodhohet

```bash
bash scripts/build-linux.sh          # gjithçka që bën CI
cargo run --release -p amm-sim --bin eng09 docs/sim/eng09-results.md
# stresi: GitHub Actions → stress → Run workflow (scale, seed_base)
```
