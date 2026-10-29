# Matrica e pranimit dhe portat e fazave

Data: 28 tetor 2026, përditësuar 29 tetor 2026 pas rishikimit të jashtëm inxhinierik (F-06). Vlerësimet janë për këtë projekt, jo për bibliotekat ose protokollet e referuara. Çdo `PASS` më poshtë lidhet me teste konkrete që ekzekutohen në CI (`scripts/build-linux.sh`) dhe në workflow-n e stresit; evidenca e përbashkët është **E-05**. Testet onchain përdorin programin real SBF në LiteSVM, jo validator; prandaj asnjë portë që kërkon validator/devnet nuk hapet nga kjo evidencë.

## Si lexohet statusi

- `NOT IMPLEMENTED`: mungon implementimi ose testi i ekzekutueshëm i kërkuar. Nuk nënkupton test të kaluar.
- `IMPLEMENTED / NOT RUN`: testi ekziston, por nuk ka evidencë ekzekutimi të lidhur me revision-in.
- `PARTIAL`: ka implementim dhe prova për një nën-bashkësi të kriterit; kriteri i plotë dhe porta mbeten të papranuara.
- `PASS`: kërkon komandën, revision-in, rezultatin dhe kufijtë e testit. Një model numerik mund të kalojë vetëm kriterin e modelit.
- `FAIL` ose `BLOCKED`: shkaku dhe prova ruhen; një portë nuk hapet duke ndryshuar etiketën.

Matrica nis konservativisht. Dokumentet arkitekturore nuk janë implementim onchain. Një rezultat lokal nuk provon paralelizëm të validatorit, integrim oracle, custody, auditim ose gatishmëri prodhimi.

Objektivi aktual është 8 shard-e fillestare dhe registry `u64` deri në 56 shard-e. Kjo është implementuar (E-05). Evidenca historike E-01 ruhet sipas testit që u ekzekutua; ajo nuk provoi as katër dhe as tetë lane onchain.

## A. Modeli dhe AMM-ja statike

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| A-01 | SEC-02 | Specifikim domain-i: decimals, fixed-point, kufij çmimi/L/tick dhe overflow. Refuzim për zero, negative dhe vlera jashtë domain-it. | PASS | E-05: [numeric-spec.md](numeric-spec.md) §1–§4; `math_vectors` (vektorë nga referenca e pavarur), `full_tick_table` (të 887 273 ticks), `math_properties`; refuzimi i zero/jashtë domain-it në `eng06_swap::swap_guards_slippage_dust_deadline_limits` dhe `eng10_attacks::extreme_amounts_and_bounds`. |
| A-02 | SEC-02/03 | Çdo swap krahasohet me referencë racionale të pavarur; output nuk tejkalon output-in ideal pas tarifës, devijimi kufizohet sipas rregullit të deklaruar të rrumbullakimit. | PARTIAL | E-05: çdo **hap** swap-i (`compute_swap_step`) krahasohet bit-për-bit me referencën racionale të pavarur (`clmm_vectors.py`, 600 raste) dhe vetitë e rrumbullakimit kontrollohen në ~61M raste. Swap-i i plotë me shumë hapa onchain krahasohet me motorin host (`eng06_swap::random_workload…`), që është i njëjti kod, jo referencë e pavarur. Mungon: referencë e pavarur për ciklin e plotë me crossing. |
| A-03 | SEC-03 | Konservim inventari për çdo mint, tarifa eksplicite, asnjë balancë negative; test për të dy drejtimet dhe sasi kufitare. | PASS | E-05: `engine_model` (ledger për çdo vault pas çdo operacioni, dy drejtimet, sasi kufitare), `eng06_swap::random_workload…` (ruajtja e token-ave onchain dhe dalja e plotë), `eng10_attacks::extreme_amounts_and_bounds`. |
| A-04 | SEC-04 | Add/remove/collect dhe kalimet në kufij tick ruajnë L dhe pretendimet; tarifa nuk merret dy herë; ciklet pa ndryshim tregu nuk prodhojnë fitim nga rrumbullakimi. | PASS | E-05: `engine_model` (L aktive dhe gross/net për çdo tick; aftësia paguese ekzakte me tarifat që nga checkpoint-i; dalje e plotë në shumë rende, F-05; round trip pa fitim), `math_vectors::fee_inside_modular_matches`, `eng06_swap::lp_fees_are_pro_rata…`, `eng10_attacks::dust_swaps_never_profit`. |
| A-05 | SEC-05 | Operacion në lane i lë byte-for-byte të pandryshuara lane-et e tjera; interleaving i lane-eve disjunkte jep rezultat të barabartë me ekzekutimet lokale. | PASS | E-05: `lane_isolation::work_on_one_shard_leaves_the_others_byte_identical` (swap, add, remove, collect në një shard; 7 të tjerët byte-për-byte të pandryshuar), `lane_isolation::interleaving_two_shards_equals_running_them_alone`, `eng03_accounts::preparing_one_shard_leaves_others_byte_identical`. |
| A-06 | SEC-02/09 | Property fuzzing me seed të ruajtur dhe gjurmë minimale për dështim; përfshin round trips, fee boundaries dhe likuiditet të ulët. | PARTIAL | E-05: property testing me seed të printuar dhe të riprodhueshëm (`AMM_STRESS_SEED`, `AMM_SVM_SEED`), stres 16+16 punë paralele; round trips, kufijtë e tarifave dhe likuiditet i ulët përfshihen. Mungon: minimizimi automatik i gjurmës (shrinking); ka vetëm `AMM_TRACE`. |
| A-07 | SEC-01 | Programi refuzon owner/seeds/lane/generation/authority/mint/token-program të gabuar dhe alias-e të palejuara. | PASS | E-05: `eng03_accounts` (role, signer, seeds, ID/generation, llogari të zëvendësuara, Token-2022 dhe token program i gabuar), `eng04_lifecycle::…bound_to_their_shard_and_owner`, `eng06_swap::swap_rejects_wrong_accounts`, `eng10_attacks::routing_output_into_a_vault_drains_nothing`, `pda_abi`. |
| A-08 | SEC-03/11 | CPI Token e vlefshme përputh delta reale me kontabilitetin; donacionet trajtohen sipas politikës; extensions të pambështetura refuzohen. | PASS | E-05: `eng06_swap::random_workload…` (delta reale e SPL përputhet me kontabilitetin në çdo swap), `eng10_attacks::vault_donation_changes_no_outcome`, `eng08_wsol` (native SOL), Token-2022 refuzohet (`eng03_accounts`). |
| A-09 | SEC-05 | Inspektim i të gjitha account metas dhe CPI-ve: tetë lane me vault-e/fee state disjunkte. Shared config/registry/program/mints vetëm readonly kur semantika e lejon. | PASS | E-05: `lane_isolation::split_swap_write_sets_are_disjoint` (në swap-in e ndarë në 8 shard-e, llogaritë writable janë disjunkte; market, registry dhe token program vetëm readonly), `idl_acceptance` (IDL i plotë i llogarive). CPI e vetme është SPL Token `transfer`. |
| A-10 | SEC-05/13 | Test ngarkese me validator: 1 kundrejt 2/4/8 lane, payer/user token accounts disjunkte; kontroll negativ me payer/vault të përbashkët. Raportohen p50/p95, sukseset, CU, tarifa, shpërndarja dhe konfigurimi. | PASS (validator lokal me një nyje) | E-06: [validator-bench.md](bench/validator-bench.md). 1 shard 923 swap/slot (36.9M CU, kufiri për llogari), 2 shard-e 1.9×, 4 dhe 8 shard-e 2.5× (kufiri i bllokut), payer i përbashkët 0.98×. 0 dështime; vonesa p50/p95 në slot-e; tarifa bazë 5 000 lamports. **Nuk** është matje mainnet. |
| A-11 | SEC-13 | Serializim/simulim i transaksionit v1 me kufij CU/loaded-data eksplicite dhe v0 kur mbështetet; numërohen të gjitha llogaritë/bajtet dhe CPI-të. | PARTIAL | E-05: [tx-budget.md](tx-budget.md) — transaksione legacy dhe v0 + ALT me swap-e reale, numërim ekzakt i llogarive dhe bajteve, CU i matur, refuzim mbi 64 llogari. Mungon: formati i transaksionit v1 dhe kufiri loaded-data i matur. **Mbetet e hapur.** |
| A-12 | SEC-09 | Min-out/max-in, price-limit dhe deadline kontrollohen në ekzekutim; teste për quote stale, renditje kundërshtare dhe kufi çmimi. | PASS | E-05: `eng06_swap::swap_guards_slippage_dust_deadline_limits` (min-out, kufiri i çmimit, deadline, dust), `engine_model` (gjendje e pandryshuar pas dështimit). |

**Porta G0 — model kërkimor:** A-01 deri A-06 kalojnë për domain-in e shpallur. Nëse modeli përdor CPMM, kalimi i tij provon vetëm CPMM; nuk quhet CLMM dhe nuk kalon kriteret tick të A-04.

**Porta G1 — MVP AMM statike në validator/devnet:** G0 dhe A-07 deri A-12 kalojnë; përshkrimi publik përcakton qartë modelin ekonomik dhe kufijtë e benchmark-ut. Pa fonde reale dhe pa pretendim auditimi. Një router, test Python ose test unit nuk zëvendëson provën e validatorit.

## R. Registry me bitmask dhe aktivizim 8 → 56

Statuset u përditësuan më 29 tetor 2026 me evidencën E-05.

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| R-01 | SEC-14 | Registry përdor `u64`; pranon vetëm bit-et 0..55, refuzon çdo bit 56..63. `count_ones()` jep numrin aktiv ≤56; count i ruajtur që nuk përputhet refuzohet. Testo 0, `0xff`, bit 55 dhe maskën e plotë të vlefshme. | PASS | E-05: E-03 (host) plus onchain `eng03_accounts::prepare_rejects_bad_ids_and_generations`, `eng07_expansion::all_56_shards_can_be_active`. |
| R-02 | SEC-14 | Guard `id < 56` ekzekutohet përpara checked shift. Testo id 0/7/8/55; refuzo secilin id 56..63 dhe vlera më të mëdha të input-it pa panic, truncate ose ndryshim gjendjeje. | PASS | E-05: E-03 (çdo `u8` në host) plus onchain `eng03_accounts::prepare_rejects_bad_ids_and_generations` (ID ≥ 56 refuzohet pa ndryshim gjendjeje). |
| R-03 | SEC-16 | Prepare për shard-et 0..7 është i versionuar/idempotent; finalize publikon atomikisht pool ready + maskën `0xff` vetëm kur të tetë janë gati. Retry/dështim në çdo hap nuk finalizon pool të pjesshëm; shard-et 8..55 nuk kërkojnë krijim ose rent fillestar. | PASS | E-05: `eng04_lifecycle::full_setup_opens_all_eight_atomically`, `open_market_requires_every_shard_ready`, `cancelled_setup_refunds_every_lp`, `eng03_accounts::prepare_same_shard_twice_fails_without_state_change`. |
| R-04 | SEC-15/17 | Activate kërkon owner/seed/pool/generation, statusin e duhur, vault-e reale dhe likuiditet/LP allocation të vlefshëm. Flag i rremë funded-ready, vault bosh ose alokim LP i papërfunduar refuzohen; biti vetëm nuk ndryshon inventarin. TVL jashtë intervalit nuk zëvendëson thellësinë aktive: testo tregti/slippage në dy drejtime, headroom dhe inventar real. Pragjet USD kërkojnë oracle të vlefshëm. | PARTIAL | E-05: `eng07_expansion::activation_rules` (role keeper, readiness me L dhe inventar real, devijimi i çmimit ndaj referencës, pauza). Mungon: pragjet në USD me oracle (jashtë v1). |
| R-05 | SEC-05/15 | Swap e lexon registry readonly; lane të ndryshme kanë W/R disjunkte përveç leximeve të përbashkëta. Activation shkruan registry/status/readiness atomikisht; garat activate/deactivate/swap provohen në të dy renditjet dhe dështimi rikthen bit-in/statusin. | PARTIAL | E-05: swap e lexon registry readonly (`lane_isolation::split_swap_write_sets_are_disjoint`); aktivizimi është atomik (`eng07_expansion`). Mungon: test garash activate/swap në validator. |
| R-06 | SEC-18 | Routing zgjedh rank modulo `count_ones(mask)` dhe pastaj bit-in aktiv; refuzon maskë zero. Testo maska sparse me boshllëqe, vetëm bit 55 dhe të gjithë bit-et e vlefshëm; nuk përdoret `hash & 55`. Onchain refuzon ID joaktiv edhe kur router-i e sugjeron. | PARTIAL | E-05: onchain refuzohet shard-i joaktiv (`eng04_lifecycle`, `eng07_expansion::activation_rules`); `select_by_hash` testohet në host (E-03). Router-i/SDK nuk është shkruar. |
| R-07 | SEC-04/17 | Shard joaktiv por me fonde lejon withdraw/collect të autorizuar sipas pretendimeve; çaktivizimi nuk ndryshon LP claims ose fonde dhe nuk lejon swap të ri. Testo disable→withdraw dhe retry pa double claim. | PARTIAL | E-05: `eng04_lifecycle::paused_market_allows_exits_only`, `cancelled_setup_refunds_every_lp`. Deactivation nuk është në v1. |
| R-08 | SEC-03/04/10/17 | Shtimi i shard-it të ri provon burimin e kapitalit dhe LP allocation pa ndryshuar pretendimet e vjetra. Split ka test veçmas për transferim/ruajtje të pretendimeve; aktivizimi me bit nuk konsiderohet split. | PARTIAL | E-05: `eng07_expansion` — shard-i i ri financohet nga LP-të e tij me kapital të ri, në vault-e të veta; shard-et ekzistuese nuk kanë llogari writable të përbashkëta me të (A-09). Mungon: test byte-për-byte i shard-eve të vjetra gjatë aktivizimit. Split nuk është implementuar. |
| R-09 | SEC-13/16 | Mat bajtet, CU dhe account metas për prepare/finalize të 8 shard-eve dhe aktivizim incremental deri në 56. Nëse finalizimi kërkon staging, witness-et e versionuara nuk mund të ripërdoren pas ndryshimit të readiness. | NOT IMPLEMENTED | Bajtet/CU për prepare/open të 8 shard-eve nuk janë matur ende (swap-i është matur në [tx-budget.md](tx-budget.md)). |

G1 për objektivin aktual kërkon edhe R-01 deri R-07 dhe R-09 për tetë shard-et fillestare, brenda veçorive të aktivizuara. Versioni i parë nuk përfshin deactivation/shrink: degët e këtyre skenarëve mbeten teste për fazën e ardhshme, ndërsa R-05 provon activation/swap dhe R-07 tërheqjet pre-activation/pause sipas politikës. Zgjerimi me shard-e të reja kërkon R-08 për kapital të ri; split mbetet i kushtëzuar nga G3 dhe nuk mund të anashkalohet me activation.

## B. Huadhënia dhe likuidimi

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| L-01 | SEC-06 | Oracle feed/program identity, timestamp/units, future skew, freshness, confidence dhe exponent/decimals testohen me raste negative. Update i nënshkruar por stale refuzohet. | NOT IMPLEMENTED | — |
| L-02 | SEC-06/07 | Shëndeti llogaritet nga gjendja e ekzekutimit; borrow/withdraw i pasigurt dhe likuidim i palejuar refuzohen; outage nuk bllokon repay pa arsye të specifikuar. | NOT IMPLEMENTED | — |
| L-03 | SEC-07 | Shlyerja, bonusi, kolaterali i marrë, close factor dhe min-out respektojnë formulat/kufijtë; testo 1 njësi bazë, dust dhe kolateral të shteruar. | NOT IMPLEMENTED | — |
| L-04 | SEC-03/07 | Repay/withdraw/liquidate në të njëjtin pozicion kanë rezultate seriale të vlefshme; çdo dështim CPI rikthen të gjithë ndryshimet, përveç tarifave të rrjetit. | NOT IMPLEMENTED | — |
| L-05 | SEC-08 | Borrow paralel respekton used_i/cap_i dhe shumën globale; quota transfer/replay/stale witness nuk dyfishon kapacitetin. | NOT IMPLEMENTED | — |
| L-06 | SEC-05 | Likuidime në lane të ndryshme përdorin rezerva/vault-e/fee receivers dhe burime bot/flashloan disjunkte; benchmark negativ provon koston e burimit të përbashkët. | NOT IMPLEMENTED | — |
| L-07 | SEC-07/09 | Skenarë price-gap, mungesë liquidator-ësh, gas/fee të larta dhe borxh të keq; humbjet regjistrohen dhe nuk maskohen si aktive të rikuperuara. | NOT IMPLEMENTED | — |
| L-08 | SEC-02/08 | Indeksi i interesit dhe debt shares përputhen me referencën; nuk ka shlyerje negative, falje borxhi nga rounding ose përdorim të indeksit të lane-it tjetër. | NOT IMPLEMENTED | — |

**Porta G2 — likuidim eksperimental:** G1 dhe L-01 deri L-08 kalojnë; parametrat e rrezikut, stres-skenarët dhe kufijtë e humbjeve dokumentohen. Asnjë premtim se sharding heq rrezikun e paaftësisë paguese.

## C. Split, merge dhe migrim dinamik

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| D-01 | SEC-10 | State machine lejon vetëm tranzicionet e shpallura; old-generation transaction refuzohet pas aktivizimit të ri; destination PDA lidhet me parent/generation. | NOT IMPLEMENTED | — |
| D-02 | SEC-03/04/10 | Para/pas split: inventari, L, LP claims, tarifa dhe borxhi ruhen me dust eksplicit; claim lazy konsumohet vetëm një herë dhe nuk shton L dy herë. | NOT IMPLEMENTED | — |
| D-03 | SEC-08/10 | Ndërprerje/retry/replay në çdo hap nuk aktivizon dy burime ose dy kuota; abort/recovery është i testuar. | NOT IMPLEMENTED | — |
| D-04 | SEC-07/10 | Swap/withdraw/liquidation kundrejt migrimit ka rezultat serial të vlefshëm; freeze ka kufi dhe rrugë rikuperimi, pa humbje të fshehura. | NOT IMPLEMENTED | — |
| D-05 | SEC-04/09 | Merge me çmime të ndryshme kërkon settlement të përcaktuar; vlera/pretendimet nuk transferohen pa rregull eksplicit midis LP-ve. | NOT IMPLEMENTED | — |
| D-06 | SEC-05/13 | Directory update, kufijtë e account metas/CU, rent dhe kosto krijimi/mbylljeje maten nën load; përfitimi neto krahasohet me lane statike. | NOT IMPLEMENTED | — |

**Porta G3 — migrim eksperimental:** G2 dhe D-01 deri D-06 kalojnë nëse produkti përfshin huadhënie; për një produkt vetëm AMM mjafton G1 plus D-01 deri D-06, me pretendimet për borxh/likuidim të përjashtuara shprehimisht. Kompleksiteti dinamik pranohet vetëm nëse përfitimi i matur justifikon koston dhe rrezikun.

## D. Para çdo përdorimi me kapital real

| ID | Rreziku | Kriteri | Statusi | Evidenca |
|---|---|---|---|---|
| P-01 | SEC-01..18 | Audit i pavarur i revision-it përfundimtar dhe korrigjim/rivlerësim i gjetjeve të rënda; auditimi i një dependency nuk vlen si audit i këtij programi. | NOT IMPLEMENTED | — |
| P-02 | SEC-12 | Upgrade authority/admin, limite fillestare, pause, monitoring dhe procedurë incidenti testohen; adresat/program hash verifikohen. | NOT IMPLEMENTED | — |
| P-03 | SEC-06/07/09 | Monitorim i oracle freshness, failed liquidations, lane utilization, vault-accounting divergence dhe bad debt me veprime operative të qarta. | NOT IMPLEMENTED | — |

**Porta G4 — shqyrtim për kapital real:** plotësohen portat për veçoritë e aktivizuara dhe P-01 deri P-03. Kalimi kërkon shqyrtim të posaçëm; ky repo nuk jep automatikisht një pretendim "production-ready".

## Regjistri i evidencës

### E-05 — programi onchain dhe rishikimi i jashtëm (ENG-03..ENG-10, F-01..F-06)

Branch `claude/project-thread-j3s3pr`, 29 tetor 2026, nga commit `a07b569` e tutje; revision-i dhe run-et e CI/stresit janë në [STATUS.md](../STATUS.md). Komanda: `bash scripts/build-linux.sh` (fmt, clippy `-D warnings`, testet host, të gjitha ticks, build SBF, IDL me IdlBuilder, testet SVM me `AMM_REQUIRE_IDL=1`) dhe workflow-i `stress`. Runtime: rustc 1.95.0 (host), 1.98.1 (SVM), Agave 4.3.2, platform-tools v1.58, LiteSVM 0.18.1.

Kufijtë: LiteSVM ekzekuton programin real SBF me runtime-in e Agave 4.3.2, por nuk është validator: nuk mat paralelizëm, rrjet, priority fees ose sjellje nën ngarkesë. Kufiri i paketës (1 232 bajt) llogaritet, nuk ekzekutohet nga rrjeti. Asnjë auditim sigurie. **Portat G1–G4 nuk janë kaluar**: A-11 dhe R-09 mbeten të hapura dhe nuk ka deploy në devnet. A-10 kalon vetëm për validator lokal (E-06).

### E-06 — benchmark-u në validator (A-10)

Commit `28debfb`, 29 tetor 2026: [bench run 1](https://github.com/intopic/1/actions/runs/40860681987) dhe [CI run 26](https://github.com/intopic/1/actions/runs/40860682035), të dy jeshil. Komanda: `bash scripts/bench-validator.sh 16 3` (workflow `bench`). Runtime: `solana-test-validator` 4.3.2 me një nyje në runner-in `ubuntu-24.04` të GitHub; 256 tregtarë me payer dhe llogari tokenash të veta; 4 096 swap-e për provë, 3 përsëritje me rend të alternuar; kontroll negativ me një payer të përbashkët. Rezultati në CI përputhet me matjen lokale (tabela në [validator-bench.md](bench/validator-bench.md)).

Kufijtë: një nyje, pa gossip, rotacion leader-ash, priority fees ose trafik tjetër. Mat sa swap-e pranon blloku nën kufijtë e kostos të Agave-s, jo kohën e ekzekutimit në thread-e. Nuk vlen si pretendim për mainnet.

### E-01 — kernel lokal virtual-reserve

Rishikuesi i sigurisë ekzekutoi `cargo test -p amm-model --offline` më 28 tetor 2026 nga rrënja `sharded_amm`: **5 tests passed, 0 failed**; 0 doc-tests. Runtime: `rustc 1.95.0 (59807616e 2026-04-14)`, Windows. SHA256 e `crates/amm-model/src/lib.rs`: `B29659155C0332E72060E30081F554DA11EED4BEE67C4A24413768EED65C5192`.

Testet: `both_directions_conserve_input_and_separate_fees`, `invalid_parameters_and_tiny_trades_are_rejected`, `overflow_and_reserve_bounds_fail_without_partial_mutation`, `proportional_routing_avoids_fragmentation_for_matching_shards`, `stale_quotes_do_not_define_a_shared_pool_execution`.

Rishikimi i API-së konfirmon gjendje private, tarifë ceil, output floor dhe commit vetëm pasi kontrollet kalojnë. `MAX_AMOUNT` nuk garanton se çdo produkt i ndërmjetëm u96×u96 hyn në u128; overflow refuzohet me gabim dhe kjo është e dokumentuar. Testi i fragmentimit është një shembull determinist, jo property test ose provë formale.

Nuk u verifikuan: referencë racionale e pavarur, fuzzing, domain i plotë CLMM/ticks/LP, katër lane si implementim, llogari/CPI/vault-e Solana, scheduler, oracle, likuidime ose migrim. **Asnjë portë G0–G4 nuk është kaluar nga kjo evidencë.** Raportet e hershme jashtë këtij projekti mbeten kontekst dhe nuk ndryshojnë automatikisht statuset.

Evidencat e reja kërkojnë ID kriteri, revision/hash, komandë, runtime/validator, rezultat dhe kufijtë e verifikimit. Ndryshimi i implementimit kërkon evidencë të re për kriteret e prekura.

### E-02 — përgatitja e mjedisit, P1-01 pjesërisht

Më 28 tetor 2026, build SBF i `env-smoke` kaloi me SDK `solana-program = 4.1.0`, Agave 4.3.2 dhe platform-tools 1.58. Artefakti ka 10,608 bajt dhe SHA256 `581e08264a75388c8b1e9c0909e7ef984439c0c1ba33e8578e5d80975c62fdb8`. Detajet e komandës dhe pengesat ruhen në [environment-report.json](environment-report.json).

Anchor native është bllokuar nga antivirusi; validator-i native nuk arriti RPC ready. Instaluesi WSL/Ubuntu kërkon restart Windows. Nuk ka ende ekzekutim end-to-end, Anchor/IDL build ose provë v1. Kjo evidencë nuk ndryshon A-07..A-12 në PASS dhe nuk kalon asnjë portë G0–G4.

Pas restart-it, WSL 3.0.3.0 u verifikua, por Ubuntu registration dështoi me `HCS_E_HYPERV_NOT_INSTALLED`. CPU VM/SLAT mbështeten; `VirtualizationFirmwareEnabled=False`. Projekti `env-anchor` dhe manifestet e tij janë përgatitur/kontrolluar sintaksisht, pa build të ekzekutuar. Statuset e portave mbeten të pandryshuara.

### E-03 — bitmap dhe lifecycle i regjistrit (ENG-02)

Commit `0c43eac`, 28 tetor 2026, Linux, rustc 1.95.0: `cargo test -p sharded-amm` → **23 passed, 0 failed**. Mbulon: `shard_bit` për çdo `u8`, refuzimin e secilit bit 56..=63, maskat 0/`0xff`/bit 55/`VALID_MASK`/sparse, rank dhe `select_by_hash` kundrejt një reference naive (20 000 maska, seed `0x5eed000000000001`), prepare/finalize/activate me refuzim pa ndryshim gjendjeje, aktivizim të dyfishtë, overflow të `registry_version`, finalizim me përgatitje ose readiness të pjesshme, zgjerim 16/32/56, dhe model të rastësishëm të lifecycle-it (seed `0xabad1dea00000056`).

Kufijtë: logjikë e pastër Rust në host. Nuk ka instruksione, lidhje llogarish, readiness reale, CPI ose validator. Asnjë portë G0–G4 nuk kalon nga kjo evidencë.

### E-04 — mjedisi Linux (ENG-00)

Commit `069a6f8`, 28 tetor 2026: `scripts/build-linux.sh` kaloi (fmt, clippy `-D warnings`, 28 teste host, tre build SBF me platform-tools v1.58, IDL për `sharded_amm` dhe `anchor_environment_smoke`); `scripts/smoke-validator.sh` konfirmoi ping/pong në validator lokal, 161 CU, transaksion legacy. Hash-et janë në `STATUS.md`. Kjo mbyll pengesën e mjedisit të E-02 për Linux; nuk provon transaksion v1, AMM ose siguri.

