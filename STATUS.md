# Gjendja aktuale

Përditësuar: 29 tetor 2026. Repository: `github.com/intopic/1`, branch `claude/project-thread-j3s3pr`.

**Vendimi i produktit:** nisje me **8 shard-e**, ID 0–7, maskë `0xff`; zgjerim me kapital të verifikuar deri në **56**, ID 0–55. Specifikimi i plotë është [docs/DEVELOPER_HANDOFF_8_SHARDS.md](docs/DEVELOPER_HANDOFF_8_SHARDS.md).

**Përmbledhje:** programi onchain ka ciklin e plotë: krijim tregu, përgatitje shard-esh, çmim fillestar, tick arrays, pozicione LP, hapje atomike e 8 shard-eve, anulim me refund, swap exact-in, tarifa me timelock, tarifa protokolli dhe zgjerim deri në 56 shard-e. Gjithçka është testuar në CI: në host, në LiteSVM me programin real SBF, dhe në një workflow stresi paralel. **Nuk ka pasur auditim sigurie.** Një rishikim i jashtëm inxhinierik (29 tetor 2026) gjeti F-01..F-06; të gjitha u adresuan (seksioni më poshtë). Nuk është bërë deploy në devnet: kjo kërkon një keypair të programit si sekret GitHub, të cilin e jep pronari. Asnjë deklaratë për throughput, paralelizëm të matur onchain, kapje MEV ose fitim nuk është provuar.

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
| A-10 Benchmark në validator | **PASS (lokal)** | [validator-bench.md](docs/bench/validator-bench.md), workflow `bench`: 1 shard 899 swap/slot, 2 shard-e 2.0×, 4–8 shard-e 2.5× (kufiri i bllokut), payer i përbashkët 0.98×; kapital total i barabartë në çdo konfigurim. Validator me një nyje, jo mainnet. |
| ADR-0005 Tarifa dinamike për shard | **PASS (implementuar, simuluar)** | `engine::dynamic_fee`, `eng11_dynamic_fee`, eng06 krahason host me onchain. Kufijtë 0,03%–0,80%, vetëm me timelock; fikur si parazgjedhje. [Rezultatet e simulimit](docs/sim/dynamic-fee-results.md). Teste manipulimi: vajtje-ardhja humb gjithmonë; sandwich-i me tarifë dinamike nuk fiton kurrë më shumë se me tarifë fikse; lëvizja dhe kthimi nuk e lënë tarifën të ngritur. |
| ADR-0006 `rebalance_pair` + pauza | **PASS** | `engine::rebalance`, `eng12_rebalance`: pa kapital, vetëm për rebalancer-at e listuar (keeper-i në krijim, ndryshim me timelock 24h), fitimi i parashikuar saktë; bit-e pauze që s'bllokojnë kurrë daljet e LP-ve, për tregun ose për një shard të vetëm (`set_shard_pause_flags`; shard-et e tjera vazhdojnë). Shard-i me swap-e të ndalura mund të rikuperohet nga rebalancer-at kundër shard-eve të hapura; rihapet vetëm me `reopen_shard`, që kërkon çmimin brenda devijimit nga mediana e ≥ 3 shard-eve të hapura. ~69k CU. |
| Bot-i i rebalancimit | **PASS (lokal)** | [rebalancer.md](docs/bench/rebalancer.md), workflow `rebalancer`: jashtë zinxhirit, programi pa ndryshim. Një snapshot për raund, `rebalance_round` me deri në 4 çifte të ndara (një për çdo çelës të listuar, secili payer i vetin), prag fitimi për token. Në validator lokal: 4 rebalancime në të njëjtin slot; hendeku 180 → 60 bps (brezi i dy tarifave 0,30%); me swap-e konkurrente çdo dështim është `RebalanceUnprofitable → Refresh` dhe bot-i nuk humb kurrë tokena, vetëm tarifën e transaksionit. Validator me një nyje, pa bot-e konkurrente. |
| ADR-0007 Router me prag | **PASS** | `crates/router`, `eng13_router`: power of two choices kundër grumbullimit dhe taksë për çdo shard shtesë (auditimi P-01/P-02). Në validator: router-i determinist 1,00×, power of two 2,43×. Urdhri i vogël shkon në 1 shard, i madhi në disa; dalja e planit = dalja onchain; `find_rebalance` jep fitimin e saktë. `recovery`: gabimet e programit klasifikohen (rifresko / shard tjetër / më shumë shard-e / ndalo / refuzo); plani i vjetër dështon me `SlippageExceeded` → `Refresh`, plani i ri zbret saktë. |
| ADR-0008 Tetë rregullat | **PASS** | [ADR-0008](docs/adr/0008-non-negotiable-rules.md): 6 rregulla mbaheshin tashmë; rregulli 4 shtoi `verify_depth` (`engine::depth`, `eng14_depth`); rregulli 8 përsëriti A-10 me kapital total të barabartë. |
| Kontrolli i transfereve | **PASS** | Çdo transfer kontrollon balancat reale pas CPI-së: burimi zbret dhe destinacioni rritet saktësisht me shumën (`TransferAmountMismatch`), si mbrojtje për çdo token që lëviz shumë tjetër. Teste host në `token_util`; i gjithë suite-i SVM kalon. ~+1,1k CU për swap. |
| Checkpoint-i i tarifave | **PASS** | Riauditimi P2-2: versioni me `ceil` humbiste mbetjen (L=3/L=1 paguante 2 në vend të 3). Tani checkpoint-i lëviz vetëm kur ndryshon likuiditeti, ndërsa `fees_credited_a/b` mban sa është kaluar te `owed`. Mbledhja pas çdo swap-i = një mbledhje në fund, saktësisht (rasti i riauditimit + 500 raste të rastit, `eng15`). Në çdo ndryshim likuiditeti humbet < 1 njësi. Position 193 bajt. |
| Riauditimi i e219657 (5 × P2, 1 × P3) | **PASS (lokal; CI në pritje)** | P2-1: `reopen_shard` merr si referenca 1–2 shard-et që tregtojnë, ose bën bootstrap kundrejt shard-eve të ndalura kur nuk tregton asnjë (`registry.swap_paused_mask`; teste me 8 dhe 56 shard-e). P2-2: shih rreshtin e mësipërm. P2-3: zgjedhja e rastit përdor 128 bit me rejection sampling, peshat mbi 2^64 dhe tejmbushja testohen. P2-4: kufiri i regret-it kontrollohet mbi output-in përfundimtar kundrejt baseline-it (planit determinist dhe shard-it më të mirë për gjithë shumën). P2-5: certifikata e thellësisë lidhet me `config_version` dhe fshihet nga çdo ndryshim likuiditeti. P3: L = 10^15 në dokument, "pa status" = rezultat i panjohur, dhe çdo tabelë mban commit-in dhe hash-in e SBF-së. |
| Krahasimi publik | **I rishkruar** | [comparison.md](docs/comparison.md): vetëm pretendime të matura, Raydium/Orca me burime në commit të fiksuar. |

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

Ekzekutimi i tretë, pas rregullimeve të rishikimit: [stress #3](https://github.com/intopic/1/actions/runs/40805983720), seed_base 3, commit `2b7f968`: **35/35 punë kaluan**. Kontrollet e reja të aftësisë paguese (F-05) dhe testet e reja SVM ishin pjesë e tij. Një punë on-chain zgjati 45 minuta në runner, ndërsa me të njëjtin seed kalon lokalisht për 19 sekonda; ajo kaloi pa gabim.

Ekzekutimi riprodhohet nga GitHub Actions → stress → Run workflow, me `scale` (shumëzues) dhe `seed_base`. Çdo punë e shtyp seed-in e vet.

## Rishikimi i jashtëm inxhinierik (F-01..F-06)

Rishikimi u bë mbi commit `0543b9b`. Rregullimet janë në commit-et `a07b569`..`2b7f968`, me CI jeshil ([run 19–24](https://github.com/intopic/1/actions/workflows/ci.yml)).

| ID | Gjetja | Rregullimi | Evidenca |
|---|---|---|---|
| F-01 | IDL pa 13 events dhe 54 errors | `tools/idl-build` me IdlBuilder të Anchor-it | `idl_acceptance` dekodon një event real vetëm nga IDL |
| F-02 | Dy ABI për PDA-në e pozicionit | ABI v1 = seeds sipas range-it; versioni në seed-in e market-it | `pda_abi`, `docs/vectors/pda_fixtures.json` (referencë e pavarur) |
| F-03 | Formula Q64 e vjetër në handoff | Q128 normative; formula e vjetër e shënuar historike | `math_vectors::{u256_limbs…, fee_inside_modular…}` |
| F-04 | "ALT zgjidh kufirin 64" | Gabim i korrigjuar: ALT zgjidh vetëm bajtet | `tx_budget`, [tx-budget.md](docs/tx-budget.md) |
| F-05 | Aftësia paguese pa tarifat që nga checkpoint-i | Invariant ekzakt + dalje në shumë rende | `engine_model` (mutacioni e rrëzon) |
| F-06 | Dokumente të vjetruara | Indeks normativ/historik, acceptance sipas kriterit, kontroll lidhjesh | [docs/README.md](docs/README.md), [acceptance.md](docs/acceptance.md) |

Ku nuk ndoqëm rekomandimin: `MAX_SWAP_STEPS` mbetet 128 dhe jo 64, sepse 3 tick arrays mund të kërkojnë ~100 hapa (ADR-0004).

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
