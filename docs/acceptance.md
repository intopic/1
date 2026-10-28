# Matrica e pranimit dhe portat e fazave

Data: 28 tetor 2026. Vlerësimet janë për këtë projekt, jo për bibliotekat ose protokollet e referuara.

## Si lexohet statusi

- `NOT IMPLEMENTED`: mungon implementimi ose testi i ekzekutueshëm i kërkuar. Nuk nënkupton test të kaluar.
- `IMPLEMENTED / NOT RUN`: testi ekziston, por nuk ka evidencë ekzekutimi të lidhur me revision-in.
- `PARTIAL`: ka implementim dhe prova për një nën-bashkësi të kriterit; kriteri i plotë dhe porta mbeten të papranuara.
- `PASS`: kërkon komandën, revision-in, rezultatin dhe kufijtë e testit. Një model numerik mund të kalojë vetëm kriterin e modelit.
- `FAIL` ose `BLOCKED`: shkaku dhe prova ruhen; një portë nuk hapet duke ndryshuar etiketën.

Matrica nis konservativisht. Dokumentet arkitekturore nuk janë implementim onchain. Një rezultat lokal nuk provon paralelizëm të validatorit, integrim oracle, custody, auditim ose gatishmëri prodhimi.

Objektivi aktual është 8 shard-e fillestare dhe registry `u64` deri në 56 shard-e. Kjo është kërkesë projektimi e paimplementuar. Evidenca historike E-01 ruhet sipas testit që u ekzekutua; ajo nuk provoi as katër dhe as tetë lane onchain.

## A. Modeli dhe AMM-ja statike

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| A-01 | SEC-02 | Specifikim domain-i: decimals, fixed-point, kufij çmimi/L/tick dhe overflow. Refuzim për zero, negative dhe vlera jashtë domain-it. | PARTIAL | E-01: model integer virtual-reserve, kufij u96/u128 dhe input zero; pa decimals/ticks/L finite. |
| A-02 | SEC-02/03 | Çdo swap krahasohet me referencë racionale të pavarur; output nuk tejkalon output-in ideal pas tarifës, devijimi kufizohet sipas rregullit të deklaruar të rrumbullakimit. | NOT IMPLEMENTED | — |
| A-03 | SEC-03 | Konservim inventari për çdo mint, tarifa eksplicite, asnjë balancë negative; test për të dy drejtimet dhe sasi kufitare. | PARTIAL | E-01: konservim brenda rezervave virtuale dhe tarifa të ndara në dy drejtime; pa mint/vault real. |
| A-04 | SEC-04 | Add/remove/collect dhe kalimet në kufij tick ruajnë L dhe pretendimet; tarifa nuk merret dy herë; ciklet pa ndryshim tregu nuk prodhojnë fitim nga rrumbullakimi. | NOT IMPLEMENTED | — |
| A-05 | SEC-05 | Operacion në lane i lë byte-for-byte të pandryshuara lane-et e tjera; interleaving i lane-eve disjunkte jep rezultat të barabartë me ekzekutimet lokale. | NOT IMPLEMENTED | — |
| A-06 | SEC-02/09 | Property fuzzing me seed të ruajtur dhe gjurmë minimale për dështim; përfshin round trips, fee boundaries dhe likuiditet të ulët. | NOT IMPLEMENTED | — |
| A-07 | SEC-01 | Programi refuzon owner/seeds/lane/generation/authority/mint/token-program të gabuar dhe alias-e të palejuara. | NOT IMPLEMENTED | — |
| A-08 | SEC-03/11 | CPI Token e vlefshme përputh delta reale me kontabilitetin; donacionet trajtohen sipas politikës; extensions të pambështetura refuzohen. | NOT IMPLEMENTED | — |
| A-09 | SEC-05 | Inspektim i të gjitha account metas dhe CPI-ve: tetë lane me vault-e/fee state disjunkte. Shared config/registry/program/mints vetëm readonly kur semantika e lejon. | NOT IMPLEMENTED | — |
| A-10 | SEC-05/13 | Test ngarkese me validator: 1 kundrejt 2/4/8 lane, payer/user token accounts disjunkte; kontroll negativ me payer/vault të përbashkët. Raportohen p50/p95, sukseset, CU, tarifa, shpërndarja dhe konfigurimi. | NOT IMPLEMENTED | — |
| A-11 | SEC-13 | Serializim/simulim i transaksionit v1 me kufij CU/loaded-data eksplicite dhe v0 kur mbështetet; numërohen të gjitha llogaritë/bajtet dhe CPI-të. | NOT IMPLEMENTED | — |
| A-12 | SEC-09 | Min-out/max-in, price-limit dhe deadline kontrollohen në ekzekutim; teste për quote stale, renditje kundërshtare dhe kufi çmimi. | PARTIAL | E-01: stale quote provohet në model; min-out, price-limit dhe deadline mungojnë. |

**Porta G0 — model kërkimor:** A-01 deri A-06 kalojnë për domain-in e shpallur. Nëse modeli përdor CPMM, kalimi i tij provon vetëm CPMM; nuk quhet CLMM dhe nuk kalon kriteret tick të A-04.

**Porta G1 — MVP AMM statike në validator/devnet:** G0 dhe A-07 deri A-12 kalojnë; përshkrimi publik përcakton qartë modelin ekonomik dhe kufijtë e benchmark-ut. Pa fonde reale dhe pa pretendim auditimi. Një router, test Python ose test unit nuk zëvendëson provën e validatorit.

## R. Registry me bitmask dhe aktivizim 8 → 56

Këto janë specifikime të reja; asnjëri nuk është test i ekzekutuar ose veçori runtime e implementuar.

| ID | Rreziku | Testi / kriteri i matshëm | Statusi | Evidenca |
|---|---|---|---|---|
| R-01 | SEC-14 | Registry përdor `u64`; pranon vetëm bit-et 0..55, refuzon çdo bit 56..63. `count_ones()` jep numrin aktiv ≤56; count i ruajtur që nuk përputhet refuzohet. Testo 0, `0xff`, bit 55 dhe maskën e plotë të vlefshme. | NOT IMPLEMENTED | — |
| R-02 | SEC-14 | Guard `id < 56` ekzekutohet përpara checked shift. Testo id 0/7/8/55; refuzo secilin id 56..63 dhe vlera më të mëdha të input-it pa panic, truncate ose ndryshim gjendjeje. | NOT IMPLEMENTED | — |
| R-03 | SEC-16 | Prepare për shard-et 0..7 është i versionuar/idempotent; finalize publikon atomikisht pool ready + maskën `0xff` vetëm kur të tetë janë gati. Retry/dështim në çdo hap nuk finalizon pool të pjesshëm; shard-et 8..55 nuk kërkojnë krijim ose rent fillestar. | NOT IMPLEMENTED | — |
| R-04 | SEC-15/17 | Activate kërkon owner/seed/pool/generation, statusin e duhur, vault-e reale dhe likuiditet/LP allocation të vlefshëm. Flag i rremë funded-ready, vault bosh ose alokim LP i papërfunduar refuzohen; biti vetëm nuk ndryshon inventarin. TVL jashtë intervalit nuk zëvendëson thellësinë aktive: testo tregti/slippage në dy drejtime, headroom dhe inventar real. Pragjet USD kërkojnë oracle të vlefshëm. | NOT IMPLEMENTED | — |
| R-05 | SEC-05/15 | Swap e lexon registry readonly; lane të ndryshme kanë W/R disjunkte përveç leximeve të përbashkëta. Activation shkruan registry/status/readiness atomikisht; garat activate/deactivate/swap provohen në të dy renditjet dhe dështimi rikthen bit-in/statusin. | NOT IMPLEMENTED | — |
| R-06 | SEC-18 | Routing zgjedh rank modulo `count_ones(mask)` dhe pastaj bit-in aktiv; refuzon maskë zero. Testo maska sparse me boshllëqe, vetëm bit 55 dhe të gjithë bit-et e vlefshëm; nuk përdoret `hash & 55`. Onchain refuzon ID joaktiv edhe kur router-i e sugjeron. | NOT IMPLEMENTED | — |
| R-07 | SEC-04/17 | Shard joaktiv por me fonde lejon withdraw/collect të autorizuar sipas pretendimeve; çaktivizimi nuk ndryshon LP claims ose fonde dhe nuk lejon swap të ri. Testo disable→withdraw dhe retry pa double claim. | NOT IMPLEMENTED | — |
| R-08 | SEC-03/04/10/17 | Shtimi i shard-it të ri provon burimin e kapitalit dhe LP allocation pa ndryshuar pretendimet e vjetra. Split ka test veçmas për transferim/ruajtje të pretendimeve; aktivizimi me bit nuk konsiderohet split. | NOT IMPLEMENTED | — |
| R-09 | SEC-13/16 | Mat bajtet, CU dhe account metas për prepare/finalize të 8 shard-eve dhe aktivizim incremental deri në 56. Nëse finalizimi kërkon staging, witness-et e versionuara nuk mund të ripërdoren pas ndryshimit të readiness. | NOT IMPLEMENTED | — |

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
