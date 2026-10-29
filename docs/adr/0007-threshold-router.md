# ADR-0007 — Router-i me prag (off-chain)

**Statusi:** PROPOSED (implementuar dhe testuar) · **Data:** 2026-10-29

## Konteksti

Me 8 shard-e, çdo urdhër duhet të zgjedhë ku ekzekutohet. Nëse çdo urdhër prek të 8 shard-et, transaksionet bllokojnë të gjitha llogaritë e tregut dhe paralelizmi i matur në A-10 (~2,5× swap-e për slot) humbet. Nëse çdo urdhër shkon në një shard të vetëm, urdhrat e mëdhenj paguajnë më shumë rrëshqitje çmimi. Përdoruesi vendosi (29 tetor 2026): urdhrat e vegjël në 1 shard, të mëdhenjtë të ndarë në pak shard-e.

## Vendimi

Crate i ri `crates/router` (`amm-router`), vetëm off-chain. Programi onchain nuk ndryshon: plani dërgohet si një transaksion me një `swap` për çdo shard.

- **Libra (`Book`).** Gjendja e shard-it dhe tick arrays të lexuara nga zinxhiri. Kuota përdor të njëjtin motor (`engine::swap_exact_in`) dhe të njëjtën tarifë (bazë ose dinamike, ADR-0005) si programi, prandaj është e saktë deri në bajt, jo një përafrim.
- **Renditja.** Çdo shard kuotohet me `amount / max_shards`. Shard-et renditen sipas daljes, me id si barazues.
- **Pragu.** Për `k = 1, 2, …` urdhri ndahet në mënyrë të barabartë mes `k` shard-eve më të mirë. `k` rritet vetëm nëse dalja rritet të paktën `min_gain_bps` (parazgjedhje 2 bps). Kufijtë: `max_shards` (parazgjedhje 8) dhe kufiri i 64 llogarive për transaksion (`docs/tx-budget.md`).
- **Kundër grumbullimit (auditimi P-01, 29 tetor 2026).** Shard-i i parë zgjidhet me "power of two choices": 2 shard-e të tërhequra rastësisht, të peshuara me likuiditetin aktiv, me seed për çdo urdhër (p.sh. `hash(user, blockhash)`), dhe merret më i miri prej tyre. Përjashtim: nëse më i miri global e kalon këtë zgjedhje me më shumë se `max_regret_bps` (1), merret më i miri global. Kështu një avantazh i vërtetë çmimi fiton gjithmonë, ndërsa diferencat prej 1 lamport-i nuk i çojnë të gjithë te i njëjti shard. `choices = 0` jep sjelljen e vjetër deterministe.
- **Kosto e ndarjes (auditimi P-02).** Kostoja e plotë e transaksionit i ngarkohet çdo shard-i që shkruan. Prandaj çdo shard shtesë duhet të shtojë `min_gain_bps + split_tax_bps` (2 + 10 bps) plus `leg_cost_out` (CU × priority fee, e llogaritur nga klienti). Me likuiditet të njëjtë, urdhrat deri në ~80 bps ndikim në një shard përdorin 1–2 shard-e. Vetëm urdhrat shumë të mëdhenj ndahen në 4–8.
- **Rebalancimi (`find_rebalance`).** Gjen shard-in më të lirë dhe më të shtrenjtë, provon fuqitë e 2-shit si sasi hyrëse, pastaj bën kërkim ternar rreth më të mirës, në të dy drejtimet. Kthen planin vetëm nëse fitimi është `≥ min_profit`. `apply_rebalance` e përditëson librin si programi. E përdor bot-i rebalancer i protokollit, sepse vetëm çelësat e listuar mund ta thërrasin `rebalance_pair` (ADR-0006).

- **Rikuperimi nga gabimet (`recovery`, ide nga SDK-ja e Meteora DLMM, 29 tetor 2026).** `decode` e kthen kodin e gabimit të programit në `AmmError`. `classify` i ndan gabimet në pesë raste:
  - `Refresh`: snapshot-i ishte i vjetër ose pa tick array-t e duhura. Përfshin `SlippageExceeded`, `PriceLimitReached`, `TickArrayMismatch`, `DeadlineExceeded`, `RoundingDustExceeded`, dhe `TickArraysExhausted` kur këmba kishte më pak se 3 array.
  - `SkipShard`: shard-i nuk ka likuiditet në atë drejtim ose nuk është aktiv. Përfshin `ZeroLiquidityGap`, `ShardNotActive`, `PriceNotInitialized`, `ShardNotReady`.
  - `Smaller`: këmba ka nevojë për më shumë hapa ose array nga sa mban një transaksion. Përfshin `TooManySwapSteps`, dhe `TickArraysExhausted` me 3 array.
  - `Halt`: tregu nuk merr swap-e. Përfshin `SwapsPaused`, `InvalidMarketStatus`, `UnsupportedGeneration`.
  - `Reject`: gjithçka tjetër. Kërkesa është e gabuar dhe po ta përsërisësh dështon njësoj.

  `replan` planifikon sërish mbi të njëjtat libra, pa shard-in e dështuar për `SkipShard` dhe pa prag ndarjeje për `Smaller`. Për `Refresh` i thotë klientit të marrë libra të rinj, për `Halt` dhe `Reject` i thotë të ndalojë. Kështu mungesa e të dhënave nuk ngatërrohet me mungesën e likuiditetit.

  `after_send` trajton një dërgim me rezultat të panjohur, p.sh. kur RPC-ja jep timeout. Klienti kontrollon statusin e nënshkrimit:
  - Nëse transaksioni ka kaluar: `Done`, urdhri nuk dërgohet më.
  - Nëse nuk shihet dhe blockhash-i është ende i vlefshëm: ridërgohen të njëjtat bajte të nënshkruara (`ResendSame`). Kopjen e dytë rrjeti e hedh poshtë.
  - Nëse nuk shihet dhe blockhash-i ka skaduar (`block_height > last_valid_block_height`), por është kontrolluar vetëm cache-i i fundit i nyjës RPC: kthehet `CheckHistory`. Një `null` atje nuk provon se transaksioni nuk u ekzekutua.
  - Vetëm nëse blockhash-i ka skaduar dhe as kërkimi në gjithë historinë (`searchTransactionHistory`) nuk e gjen, lejohet kuotë e re (`Expired`).
  - Nëse ka dështuar: veprohet sipas `classify`. Kodet e panjohura dhe gabimet që nuk vijnë nga programi kthejnë `Reject`.

  Kjo e ngushton dritaren për një tregti të dyfishtë, por nuk e bën të pamundur, p.sh. nëse RPC-ja jep përgjigje të gabuar. Këtë do ta garantonte vetëm një identifikues urdhri që konsumohet onchain. Ai nuk është implementuar, sepse ka kosto shtesë.

Router-i nuk ka privilegj dhe nuk është i detyrueshëm. Kushdo mund të ndërtojë transaksionin vetë.

## Alternativat e refuzuara

- **Router onchain.** Do të kërkonte të gjitha shard-et si llogari në çdo transaksion, pikërisht atë që sharding-u shmang.
- **Ndarje optimale (pjesë të pabarabarta).** Fiton pak mbi ndarjen e barabartë kur shard-et janë afër njëri-tjetrit, që është gjendja që `rebalance_pair` e mban. Mund të shtohet më vonë pa ndryshuar programin.
- **Gjithmonë 8 shard-e.** Humb paralelizmin (A-10) dhe kushton më shumë CU.

## Evidenca

- `amm-router` (11 teste host; 4 për `recovery`: çdo kod deshifrohet në gabimin e vet sipas `errors.rs`, mungesa e të dhënave ndahet nga mungesa e likuiditetit, `replan` shmang shard-in e dështuar dhe ndalon kur s'ka çfarë provojë, `TooManySwapSteps` e shpërndan urdhrin në më shumë shard-e). Me 1 000 klientë dhe të njëjtin snapshot, varianti determinist i çon të gjithë te shard-i 0, ndërsa power of two i shpërndan 60–200 për shard. Një shard pak më i mirë merr rreth 2/8 e urdhrave, jo të gjithë. Një shard qartë më i mirë fiton gjithmonë, dhe një shard qartë më i keq nuk zgjidhet kurrë. Me taksën e ndarjes, urdhrat deri në 2^32 mbeten në ≤ 2 shard-e.
- Testet e mëparshme: urdhër i vogël në 1 shard dhe i madh në disa; numri i shard-eve rritet me madhësinë dhe bie kur pragu rritet; urdhri i vogël zgjedh shard-in më të lirë; `find_rebalance` mbyll diferencën.
- `eng13_router` (3 teste onchain, programi SBF i vërtetë). Një plan i bërë mbi snapshot të vjetër, pasi dikush tjetër ka tregtuar në të njëjtin shard, dështon me `SlippageExceeded`. Kodi deshifrohet dhe klasifikohet si `Refresh`, dhe plani i ri pas rifreskimit merr saktësisht daljen e kuotuar. Testet e tjera: për 1e6 dhe 4e10 në të dy drejtimet, tregtari merr saktësisht daljen e planit dhe çdo shard përfundon byte-për-byte ku thonë librat e router-it. Urdhri i vogël përdor 1 shard, i madhi disa. `find_rebalance` pas një swap-i të madh në shard-in 5 gjen `hi = 5`, dhe rebalancer-i i listuar, pa token, merr saktësisht fitimin e parashikuar.
- Cilësia e çmimit (`power_of_two_gives_up_at_most_the_regret_bound`, host). Testi përdor 40 botë me 8 shard-e, ku çmimet janë larguar nga tregti të rastit, dhe 2 000 urdhra të rastit. Krahasuar me më të mirin nga të 8 shard-et, power of two jep mesatarisht −0,004 bps, pra praktikisht njësoj. Rasti më i keq është 0,39 bps, nën kufirin `max_regret_bps`.
- Benchmark-u në validator me router-in real ([validator-bench.md](../bench/validator-bench.md)): router-i determinist 1,00×, power of two 2,30× në CI (`c2cb221`; 2,43× në matjen e parë lokale), round-robin 2,50×.

Riauditimi i 29 tetorit (e219657):
- **P2-3:** zgjedhja e rastit merrte vetëm 64 bit. Me pesha mbi 2^64 zgjidhej gjithmonë i pari: me 8 shard-e të barabarta me L = 2^65, të 100 seed-et shkuan te shard-i 0. Tani vlera e rastit merret me 128 bit dhe rejection sampling, dhe peshat zhvendosen nëse shuma kalon 128 bit. Teste: `sampling_is_fair_above_64_bits`, që mbulon 8 × 2^65, pesha 1:3 mbi 2^66, renditje të kundërt dhe tejmbushje.
- **P2-4:** `max_regret_bps` kontrollohej mbi një probe, jo mbi urdhrin e plotë, dhe rezultati mund të ishte 4,87 bps më keq. Tani `plan` e krahason output-in përfundimtar me baseline-in, që është më i miri nga plani determinist dhe shard-i më i mirë për gjithë shumën. Nëse e kalon kufirin, kthen baseline-in. Test: `final_output_stays_within_the_regret_bound`, me 30 botë me thellësi të ndryshme dhe 1 200 urdhra; testi rrëzon versionin e vjetër.

Ende pa bërë: simulim i kostos së tregtarit dhe i numrit mesatar të shard-eve për urdhër me router-in dhe rebalancer-in tonë.
