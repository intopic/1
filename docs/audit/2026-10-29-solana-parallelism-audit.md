# Auditim: si i shfrytëzojnë 8 shard-et paralelizmin e Solana-s

Data: 29 tetor 2026. Revizioni i audituar: `4e453dc` (dega `claude/project-thread-j3s3pr`). Statusi: **auditim i pavarur, vetëm lexim**. Asnjë ndryshim kodi nuk u bë në degën e punës. Nuk është auditim sigurie i plotë për kapital real.

Burimet: kodi në `programs/sharded-amm/src`, `crates/router`, `tests/svm`, matjet në [bench/validator-bench.md](../bench/validator-bench.md) dhe [tx-budget.md](../tx-budget.md), simulimi në [sim/eng09-analysis.md](../sim/eng09-analysis.md).

## 0. Përmbledhje

1. **Kontrata është e pastër për paralelizëm.** Në `swap_exact_in` nuk ka asnjë llogari writable të përbashkët mes shard-eve. `market` dhe `registry` janë vetëm-lexim, tarifat e protokollit dhe gjendja e tarifës dinamike janë brenda `ShardState`, nuk ka oracle dhe ngjarjet shkojnë në log. Swap-et në dy shard-e të ndryshëm nga dy përdorues të ndryshëm nuk konfliktojnë.
2. **Rreziku kryesor është jashtë kontratës, te router-i dhe klientët.** Router-i zgjedh shard-in "më të mirë" në mënyrë deterministe, me id si barazues ([`crates/router/src/lib.rs:287`](../../crates/router/src/lib.rs)). Kur shard-et janë në të njëjtin çmim, gjë që rebalancer-i e synon, të gjithë klientët me të njëjtin snapshot zgjedhin të njëjtin shard. Kjo i kthen 8 shard-et në 1 (P-01).
3. **Swap-i i ndarë në shumë shard-e e konsumon kapacitetin.** Kostoja e plotë e një transaksioni i ngarkohet *çdo* llogarie writable. Një urdhër i ndarë në 8 shard-e (~281k CU) harxhon 281k nga buxheti i secilit shard. Nëse të gjithë urdhrat ndahen në 8, tregu pranon rreth **142 urdhra/slot, 7.7 herë më pak se një pishinë e vetme Orca/Raydium** (~1 088) (P-02).
4. **Përfitimi real kundrejt Orca/Raydium nuk është 8×.** I matur është ~2.5× swap/slot në ngopje, sepse kufiri bëhet blloku dhe jo pishina, plus vonesë më e ulët në ngarkesë. Ky përfitim vlen vetëm kur urdhrat e vegjël shpërndahen në shard-e të ndryshëm dhe nuk ndahen në shumë shard-e. Kostot reale janë likuiditeti i ndarë, arbitrazhi mes shard-eve dhe sandwich më i lirë për shard.
5. **Ide që kanë kuptim vetëm me 8 shard-e:** zgjedhje "power of two choices" kundër grumbullimit, 4 rebalancer-a paralelë mbi çifte të ndara (8 = 4 çifte, 7 raunde mbulojnë të 28 çiftet), ndalim për shard (blast radius 1/8), tarifë kundër sandwich-it që i bie kryesisht sulmuesit, dhe eksperiment A/B live i tarifave brenda të njëjtit treg.

## 1. Çfarë janë "8 thread-et" në Solana

Shard-et nuk janë thread-e CPU. Janë **8 domene të pavarura kyçjesh (lock domains)**. Validatori i cakton transaksionet që nuk konfliktojnë te thread-et e veta të ekzekutimit:

- **Agave (banking stage + central scheduler):** një numër i vogël worker-ash për transaksione jo-votimi, rreth 4 si parazgjedhje dhe i konfigurueshëm. Scheduler-i ka kyçje "thread-aware": një transaksion mund të shkojë te një worker vetëm nëse llogaritë e tij writable janë të lira ose të kyçura nga i njëjti worker.
- **Firedancer/Frankendancer (`pack` + bank tiles):** numri i bank tiles është i konfigurueshëm. Rregulli i konfliktit është i njëjti.
- **Konflikt:** dy transaksione A dhe B konfliktojnë nëse `W_A ∩ (R_B ∪ W_B) ≠ ∅` ose anasjelltas (shih [security.md](../security.md)). Read-read nuk konflikton kurrë.
- **Kufiri i kostos për llogari:** Agave i shton kostot e transaksionit çdo llogarie writable. Me SIMD-0286 kufijtë janë 100M CU për bllok dhe 40M për llogari, pa të 60M dhe 24M. Ky është kufiri që e ndalon një pishinë të vetme te 40% e bllokut, siç e tregon benchmark-u A-10.

Prandaj me 4 worker-a, 8 shard-e prapë kanë vlerë. Kufiri që lidh një pishinë të vetme nuk është numri i thread-eve. Është kufiri i CU-ve për llogari writable dhe radha e kyçjeve. 8 shard-e e shumëzojnë kufirin për llogari me 8, derisa të arrihet kufiri i bllokut. Rreth 2.5 shard-e mjaftojnë për ta mbushur bllokun (100/40). Shard-et e tjerë ndihmojnë në shpërndarje, izolim dhe vonesë, jo në tavanin e bllokut.

Verifikim nga matja: 40M / 923 swap = **43 337 CU kosto për transaksion**, ndërsa CU të konsumuara janë ~40 000. Diferenca prej ~3.3k përputhet me koston e nënshkrimit (720) + 300 CU për çdo llogari writable (7 × 300) + të dhënat e instruksionit. Pra kufiri për llogari numëron koston e plotë të transaksionit, jo vetëm ekzekutimin. Kjo është baza e P-02.

## 2. Inventari R/W për çdo instruksion

Legjenda: **W** writable, R vetëm-lexim, S nënshkrues. "Global" do të thotë që llogaria preket nga çdo shard.

| Instruksioni | Llogari globale | Llogari për shard | Llogari të përdoruesit | Konflikt mes shard-eve? |
|---|---|---|---|---|
| `swap_exact_in` | `market` R, `registry` R, token program R | `shard` W, `vault_a` W, `vault_b` W, `vault_authority` R, 1–3 tick arrays W | payer W S, `user_token_in` W, `user_token_out` W | **Jo.** Vetëm payer-i dhe ATA-të e të njëjtit përdorues serializojnë. |
| `open_position` | `market` R | `shard` R | `owner` S, payer W S, `position` W (init) | Jo. Edhe me swap-et e shard-it konflikton vetëm read-write me `shard`. |
| `increase/decrease_liquidity`, `collect_fees` | `market` R | `shard` W, vaults W, 1–2 tick arrays W, `vault_authority` R | `position` W, ATA-të W | Jo mes shard-eve. Po me swap-et e të njëjtit shard, njësoj si te Orca/Raydium. |
| `close_position` | – | – | `owner` W S, `position` W | Jo. |
| `init_tick_array` | `market` R | `shard` R, `tick_array` W (init) | payer W S | Jo mes shard-eve. |
| `rebalance_pair` | `market` R, `registry` R | **2 shard-e** × (shard W, 2 vaults W, auth R) + tick arrays W | `caller` S, `caller_token` W, payer W | **Po, për 2 shard-et** (me qëllim). Rrezik nëse bot-i përdor një payer/ATA (P-04). |
| `collect_protocol_fees` | `market` R | shard W, vaults W | `destination_a/b` W | Destinacionet janë të përbashkëta, por ky është operacion i rrallë. |
| `set_paused`, `set_pause_flags`, `update_fees`, `apply_fees`, `update/apply_rebalancers` | **`market` W** | – | – | **Barrierë globale:** bllokon çdo swap në çdo shard për atë moment. Pranohet, sepse janë të rralla. |
| `prepare_shard`, `activate_shard` | `market` R, **`registry` W** | shard i ri W | payer W | **Barrierë globale** (registry lexohet nga çdo swap). Të rralla. |
| `open_market` | **`market` W, `registry` W** | 8 shard-e W | – | Një herë, para tregtimit. |
| `cancel_setup` | **`market` W** | – | – | Vetëm gjatë `Initializing`. |

**Llogari të fshehura që u kontrolluan dhe janë në rregull:**

- **Tarifat e protokollit:** grumbullohen në `ShardState.protocol_fees_a/b`, jo në një llogari treasury. ✔
- **Tarifa dinamike (ADR-0005):** gjendja është në `ShardState.dynamic_fee`, parametrat në `market`, që është R. ✔
- **Oracle / TWAP:** nuk ka. ✔ Nëse shtohet më vonë, duhet të jetë për shard (shih rregullin R-2 më poshtë).
- **Ngjarjet:** `emit!` shkon në log dhe nuk shton llogari. Nëse kalohet te `emit_cpi!`, `event_authority` shtohet si R, që është në rregull. ✔
- **Clock:** `Clock::get()` është syscall, jo llogari. ✔
- **Vault-et:** krijohen me seeds `["vault", shard, mint]` dhe kontrollohen me `has_one`, kështu që asnjë vault nuk ndahet. ✔
- **Token program:** vetëm-lexim. ✔

## 3. Gjetjet

Ashpërsia ka të bëjë me paralelizmin dhe ekonominë. Asnjë gjetje nuk lejon vjedhje fondesh.

### P-01 — E LARTË: router-i determinist i çon të gjithë te i njëjti shard (thundering herd)

- **Ku:** `crates/router/src/lib.rs:287` rendit sipas `amount_out` dhe pastaj sipas indeksit. `plan()` zgjedh gjithmonë shard-in e parë në renditje për urdhrat e vegjël.
- **Pse dëmton:** të gjithë klientët lexojnë të njëjtin snapshot, 1–2 slot të vjetër. Ose fiton i njëjti shard për 1 lamport diferencë, ose barazimi e çon te shard-i me id më të vogël. Të gjithë urdhrat e vegjël të atij momenti shkojnë te i njëjti `ShardState`, serializohen dhe godasin kufirin 40M të atij shard-i. Shard-et e tjerë mbeten bosh. Pas disa slot-eve "fituesi" bëhet më i keqi dhe tufa lëviz bashkë te tjetri. Benchmark-u A-10 e shmangu këtë sepse i shpërndau swap-et me round-robin, jo me router-in.
- **Ndreqja (vetëm off-chain):**
  1. **Power of two choices:** zgjidh 2 shard-e rastësisht dhe merr më të mirin. Teoria e balancimit të ngarkesës tregon se kjo e ul ngarkesën maksimale në mënyrë eksponenciale kundrejt zgjedhjes së "më të mirit global", me vetëm 2 kuota.
  2. Ose zgjidh rastësisht, të peshuar me likuiditet, mes shard-eve që janë brenda `ε` bps nga më i miri (p.sh. ε = 1 bps ose sa tarifa e një kalimi tick-u).
  3. Burimi i rastësisë mund të jetë `hash(user_pubkey, recent_blockhash)`: i riprodhueshëm për debug dhe i ndryshëm për çdo përdorues.
- **Test që duhet shtuar:** 1 000 klientë sintetikë me të njëjtin snapshot. Mat shpërndarjen mbi shard-e (tani 100% te një shard) dhe swap/slot në validator me router-in real, jo me round-robin.

### P-02 — E LARTË: swap-i i ndarë në shumë shard-e e shumëfishon koston për llogari

- **Mekanizmi:** kostoja e plotë e transaksionit i ngarkohet çdo llogarie writable (§1). Një urdhër në `k` shard-e kushton `C_k` dhe kyç `k` shard-e për gjithë kohëzgjatjen e transaksionit.
- **Numrat** (CU të matura në [tx-budget.md](../tx-budget.md), K = 1 tick array, + 720 për nënshkrimin + 300 për çdo llogari writable; 8 shard-e në grupe të ndara me `8/k` grupe; kufij 100M/40M):

| shard-e për urdhër (k) | kosto/tx (CU) | urdhra/slot, kufiri i bllokut | urdhra/slot, kufiri për llogari | **urdhra/slot** | kundrejt 1 pishine (1 088) |
|---:|---:|---:|---:|---:|---:|
| 1 | 36 741 | 2 721 | 8 709 | **2 721** | 2.5× |
| 2 | 71 701 | 1 394 | 2 231 | **1 394** | 1.3× |
| 4 | 141 608 | 706 | 564 | **564** | 0.52× |
| 8 | 281 428 | 355 | 142 | **142** | **0.13×** |

  Me kufijtë 60M/24M raportet janë të njëjta (1 633 / 836 / 338 / 85 kundrejt 653).
- **Përfundimi:** ndarja në 8 shard-e e jep çmimin e një pishine të vetme (ENG-09), por në ngopje është **rreth 8 herë më e keqe** se një pishinë e vetme Orca/Raydium për numrin e urdhrave. `min_gain_bps = 2` i router-it e mat vetëm daljen dhe e injoron këtë kosto.
- **Ndreqja:**
  - Në pragun e router-it shto një term kostoje: `gain_bps(k) > α · (k−1) · CU_per_leg · priority_fee / vlera_e_urdhrit`, plus një "taksë paralelizmi" të konfigurueshme. Kufiri praktik duhet të jetë `k ≤ 2` për shumicën e urdhrave dhe `k ≥ 4` vetëm për urdhra shumë të mëdhenj.
  - Për urdhra shumë të mëdhenj ofro mënyrën **jo-atomike**: `k` transaksione me një shard secili, secili me `min_out` të vet. Ato serializohen vetëm mbi ATA-të e të njëjtit përdorues, ndërsa shard-et lirohen menjëherë dhe swap-et e të tjerëve ndërthuren mes tyre. Rreziku i mbushjes së pjesshme duhet t'i shfaqet përdoruesit.

### P-03 — MESATARE: kërkesa e CU-ve dhe kostoja e llogarive të ngarkuara

- Swap-i konsumon ~34–40k CU. Pa `SetComputeUnitLimit`, kostoja parazgjedhje është 200k për instruksion. Scheduler-i rezervon CU-të e kërkuara para ekzekutimit dhe prioriteti llogaritet si tarifë për CU të kërkuar. Një klient që nuk e vendos limitin duket 5× më i shtrenjtë dhe zë 5× më shumë nga buxheti i shard-it derisa të ekzekutohet.
- **Ndreqja në SDK:** `SetComputeUnitLimit = simulim × 1.1` (benchmark-u përdor 60k, mund të shkojë në ~45k). Mat edhe `SetLoadedAccountsDataSizeLimit`: në disa versione kostoja e të dhënave të ngarkuara llogaritet nga limiti i kërkuar, jo nga madhësia reale. Ky është pikë për t'u matur, jo e provuar këtu.

### P-04 — MESATARE: rebalancer-i mund të serializojë gjithçka nëse përdor një payer dhe një ATA

- `rebalance_pair` kyç 2 shard-e (me qëllim), por edhe payer-in dhe `caller_token` të bot-it. Nëse bot-i dërgon disa çifte me të njëjtin payer/ATA, ato serializohen mes tyre dhe me çdo swap në të 2 shard-et e secilit çift.
- `find_rebalance` zgjedh gjithmonë çiftin më i lirë–më i shtrenjtë. Kur një shard devijon shpesh, ai bëhet "hub" që bllokohet në çdo slot.
- **Ndreqja:** përdor 4 çelësat e lejuar (`MAX_REBALANCERS = 4`) si **4 bot-e me payer dhe ATA të veçantë**, mbi 4 çifte të ndara në çdo slot (ideja I-2). Vendos `min_profit` mbi koston e transaksionit dhe të priority fee.

### P-05 — MESATARE (ekonomike): sandwich më i lirë për shard

- Një urdhër në 1 shard sheh 1/8 e likuiditetit. Sulmuesi ka nevojë për 1/8 e kapitalit për ta lëvizur çmimin njësoj, dhe dëmi për viktimën është më i madh për të njëjtën `min_amount_out`.
- **Ndreqja:** SDK me slippage të ngushtë si parazgjedhje (të llogaritur nga kuota e shard-it të zgjedhur, jo e tregut të plotë) dhe ideja I-4. Termi i ngarkesës së tarifës dinamike (`load_coeff_ppm`, `engine/dynamic_fee.rs:53`) e ndëshkon edhe viktimën, sepse ajo është swap-i i 2-të në slot. Prandaj vetëm ai nuk mjafton.

### P-06 — E ULËT: barrierat globale

- Çdo instruksion që shkruan `market` ose `registry` (§2) ndalon çdo swap në çdo shard për atë moment. Sot janë të gjitha të privilegjuara dhe të rralla, prandaj kjo pranohet.
- **Rregulli R-1:** asnjë instruksion i ardhshëm i hot path-it (swap, LP, rebalance) nuk duhet ta marrë `market` ose `registry` si W. Shto një test në CI që lexon IDL-në dhe dështon nëse `market`/`registry` janë `writable` në këto instruksione.
- **Rregulli R-2:** asnjë instruksion i hot path-it nuk duhet të **lexojë** shard-e të tjerë. Një read-lock mbi shard-in j konflikton me çdo swap që e shkruan j. Një tarifë dinamike që shikon "dispersionin mes shard-eve" ose një oracle "mesatare e 8 shard-eve" onchain do ta prishte paralelizmin. Këto llogariten off-chain.
- Operacionet e keeper-it (`prepare_shard`, `activate_shard`) duhet të bëhen në orë me pak trafik dhe me priority fee të lartë, që të mbarojnë shpejt.

### P-07 — E ULËT: nuk ka ndalim për një shard të vetëm

- `pause_flags` është për gjithë tregun (`state/market_config.rs:46`) dhe nuk ka instruksion për çaktivizim/ndalim të një shard-i (`grep deactivate` = 0). Nëse gjendet një gabim që prek vetëm një shard (p.sh. tick arrays të prishura), ndalet i gjithë tregu.
- **Ndreqja:** shih ideja I-3.

### P-08 — INFO: kontrollet e sigurisë që u verifikuan

- Swap: `shard has_one market, vault_a, vault_b`; `vault_authority` me seeds nga shard-i; mint-et e ATA-ve sipas drejtimit; tick arrays: pronari është programi, discriminator, `shard` i njëjtë, writable, pa dublikata, varg i njëpasnjëshëm nga tick-u aktual (`engine/swap.rs: check_array_sequence`). ✔
- Rebalance: `shard_x ≠ shard_y`, të dy `has_one market`, vault-et me constraint, rebalancer-i në listë, `profit > 0 && ≥ min_profit`, çmimet konvergjojnë dhe nuk kryqëzohen. `caller_token` nuk kontrollohet për pronarin, por fitimi shkon ku do rebalancer-i i besuar. Nëse `caller_token` është vetë një vault, ai është dhuratë për LP-të, jo humbje. ✔
- LP-ja nuk bllokohet kurrë në dalje (ADR-0002). Tarifat e protokollit janë të ndara nga principal-i. ✔
- `initialize_market` është pa leje. Kushdo mund të krijojë një treg të rremë me shard-e të vetat. Router-i/SDK duhet të ketë **allowlist të adresës së `market`**, sepse kontrata kontrollon vetëm konsistencën brenda tregut të dhënë.
- Logjet: një swap në 8 shard-e prodhon 8 ngjarje + 16 CPI token. Mat që nuk afrohet kufirit të logjeve (10 KB), përndryshe indeksuesit humbin ngjarje (ose kalo te `emit_cpi!`).

## 4. Përfitimi real kundrejt Orca/Raydium

| Dimensioni | Orca Whirlpool / Raydium CLMM (1 pishinë) | Ne (8 shard-e) | Statusi |
|---|---|---|---|
| Tavani i swap/slot në ngopje | ≤ 40% e bllokut (kufiri për llogari) | deri në 100% e bllokut: **~2.5×** | **I matur** (A-10, validator lokal) |
| Vonesa në ngarkesë | p50 3 slot | p50 2 slot | I matur lokalisht |
| Tregu lokal i tarifave | një radhë për të gjithë | 8 radhë, nëse klientët shpërndahen (P-01) | **I pamatur.** Pritet tarifë prioriteti më e ulët gjatë shpërthimeve. |
| Çmimi për urdhër të vogël | i gjithë likuiditeti | 1/8 e likuiditetit, impakt ~8× | **Humbje** (ENG-09: 21 → 98 bps me 1 shard) |
| Çmimi për urdhër të madh | i gjithë likuiditeti | i njëjtë me ndarje në 8 | Barazim, por me koston P-02 |
| Kapaciteti me ndarje në 8 | – | 0.13× e një pishine | **Humbje** në ngopje (P-02) |
| Arbitrazhi | 1 pishinë | 8 + arbitrazh mes shard-eve | Humbje për përdoruesin, fitim për rebalancer-in/LP-të |
| Izolimi i rrezikut | një gabim prek gjithë TVL-në | mund të kufizohet në 1/8 | **Potencial** (kërkon I-3) |
| Agregatorët (Jupiter) | 1 pishinë e thellë | 8 pishina të cekëta | Rrezik: agregatori mund ta nënvlerësojë thellësinë nëse nuk i ndan urdhrat |

**Kujdes me krahasimin:** në mainnet SOL/USDC tashmë ka disa pishina Orca/Raydium paralele (nivele të ndryshme tarifash), dhe Jupiter i ndan urdhrat mes tyre. Pra "shumë pishina për një çift" ekziston de facto. Dallimi ynë i vërtetë nuk është numri i pishinave. Është se 8 shard-et janë **të njëjtat parametra, një qeverisje, një regjistër, rebalancim pa kapital nga protokolli dhe një router që di për to**.

**Formulimi i lejuar publikisht** (në përputhje me vendimet e 29 tetorit): "~2.5× swap/slot me 8 shard-e kundrejt 1, validator lokal, kur urdhrat e vegjël shpërndahen mbi shard-e". Jo "8×", jo "100% paralel".

## 5. Ide inovative që kanë kuptim vetëm me 8 shard-e

Të renditura sipas raportit vlerë/kosto. Asnjë nuk kopjon kod Orca. Ato me shenjën ⚙ kërkojnë ndryshim në program dhe ADR.

### I-1. Router "power of two choices" + kosto paralelizmi (off-chain, kosto e ulët)

Ndreq P-01 dhe P-02 bashkë: zgjidh `d = 2` shard-e rastësisht (të peshuar me likuiditet aktiv), kuoto vetëm ato dhe merr më të mirin. `k` rritet vetëm nëse fitimi kalon pragun e kostos nga P-02. Rezultati: ngarkesa shpërndahet vetvetiu, kuotimi kushton 2 simulime në vend të 8, dhe arbitrazhi mes shard-eve bie, sepse rrjedha e porosive është e balancuar.

### I-2. Rebalancim paralel me "round-robin tournament" (off-chain, kosto e ulët)

8 shard-e = **4 çifte të ndara** në çdo slot. Me 4 çelësat `MAX_REBALANCERS`, secili me payer dhe ATA të vetin, 4 `rebalance_pair` ekzekutohen paralelisht pa konflikt mes tyre. Skema rrethore (1-faktorizim i K₈) mbulon të 28 çiftet në 7 raunde. Në çdo slot bot-i zgjedh përputhjen me fitimin maksimal (maximum-weight matching mbi 8 nyje, i lirë për t'u llogaritur). Kështu 8 çmimet konvergjojnë në pak raunde paralele, në vend të një vargu serial "më i lirë–më i shtrenjtë" (numri i saktë i raundeve duhet simuluar).

### I-3. ⚙ Ndalim për shard dhe kufi TVL për shard (blast radius 1/8)

`set_shard_paused(id)` shkruan **vetëm `ShardState.status`**, jo `market` ose `registry`. Kështu nuk krijon barrierë globale (R-1). LP-ja mbetet gjithmonë e lirë të dalë. Bashkë me një `max_liquidity` për shard gjatë fazës së parë në mainnet, një gabim në matematikën e një shard-i ekspozon ≤ 1/8 e TVL-së. Orca/Raydium nuk mund ta bëjnë këtë brenda një pishine.

### I-4. ⚙ Tarifë "reversal" për slot, kundër sandwich-it, që i bie sulmuesit

Ruaj në `ShardState.reserved` (34 bajt të lirë) `slot_start_sqrt_price` (16 B) dhe drejtimin e swap-it të parë në slot. Një swap **në drejtim të kundërt** brenda të njëjtit slot dhe shard (back-run-i i një sandwich-i) paguan tarifë shtesë proporcionale me lëvizjen e çmimit që nga fillimi i slot-it. Ideja e "anti-sandwich" ekziston si koncept te hook-et e Uniswap v4. Ajo që e bën më të fortë këtu është I-1: router-i i shpërndan përdoruesit e ndershëm, kështu që dy përdorues të ndershëm rrallë takohen në të njëjtin shard dhe slot, ndërsa sulmuesi **duhet** të godasë shard-in e viktimës. Arbitrazherët që e kthejnë çmimin paguajnë gjithashtu, gjë që pranohet, sepse tarifa shkon te LP-të. Kërkon simulim (ENG-09) para implementimit.

### I-5. ⚙ Eksperiment A/B live brenda tregut (ADR-0004)

ADR-0004 kërkon krahasimin e 5/10/30 bps dhe spacing 10/60. Me 8 shard-e mund ta bësh **në të njëjtin treg, në të njëjtën kohë, me të njëjtën rrjedhë**: p.sh. shard-et 0–3 me 5 bps dhe 4–7 me 30 bps, me tarifë bazë për shard (override nën timelock-un ekzistues). Kjo heq problemin kryesor të eksperimenteve me pishina të ndara: periudha dhe rrjedha të ndryshme. Router-i duhet ta raportojë cilin shard zgjodhi dhe pse.

### I-6. "Shtëpia" e LP-së dhe shkallë LP (SDK)

SDK-ja i ndihmon LP-të të shpërndajnë depozitën në të 8 shard-et sipas mungesës së likuiditetit (deficiti kundrejt mesatares), që thellësia për shard të mbetet e barabartë. Kjo është kushti në të cilin ndarja e barabartë e router-it është optimale (ADR-0007). Shpërndarja e pabarabartë e LP-ve e prish edhe P-01 edhe I-1.

### I-7. Telemetri paralelizmi si produkt

Publiko për çdo slot: shard-et e prekur, swap-et për shard, CU për shard dhe dispersionin e çmimeve. Kjo është provë publike, e matshme, e përfitimit (ose mungesës së tij) dhe i jep router-it/agregatorëve sinjalin për ngarkesën. Llogaritet off-chain nga ngjarjet, nuk kushton asgjë onchain.

## 6. Çfarë duhet matur më pas (para pohimeve publike)

1. **Benchmark me router-in real:** zëvendëso round-robin-in e A-10 me `plan()` tani dhe pastaj me I-1. Pritet: tani ~1× (tufa), me I-1 ~2.5×.
2. **Benchmark me ndarje:** përzierje 90% urdhra me k = 1, 10% me k = 8. Mat swap/slot dhe vonesën e urdhrave të vegjël. Kjo e vërteton tabelën e P-02.
3. **CU dhe të dhëna të ngarkuara:** swap me `SetComputeUnitLimit` 45k dhe me/pa `SetLoadedAccountsDataSizeLimit`. Mat koston në bllok (P-03).
4. **Test IDL në CI** që `market`/`registry` janë R në swap, LP dhe rebalance (R-1).
5. **Simulim ENG-09 me sandwich** dhe me I-4, para çdo ndryshimi në program.
6. **Devnet/mainnet-fork me priority fees** për tregun lokal të tarifave. Ky është i vetmi vend ku tregohet nëse 8 radhët japin tarifa prioriteti më të ulëta.

## Kufijtë e këtij auditimi

- Nuk u ekzekutua validator. Numrat e P-02 janë llogaritur nga CU-të e matura në `tx-budget.md` dhe nga modeli i kostos së Agave (720 CU për nënshkrim, 300 CU për çdo llogari writable). Modeli përputhet me A-10 (43.3k kosto/tx = 40M/923).
- Numri i worker-ave dhe detajet e scheduler-it ndryshojnë mes versioneve të Agave dhe Firedancer. Përfundimet varen vetëm nga rregulli i konfliktit dhe nga kufiri për llogari, që janë të qëndrueshme.
- Nuk është auditim i plotë i matematikës CLMM. Ai mbetet A-02 dhe auditimi i jashtëm para kapitalit real.
