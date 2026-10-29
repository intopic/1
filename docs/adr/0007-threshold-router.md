# ADR-0007 — Router-i me prag (off-chain)

**Statusi:** PROPOSED (implementuar dhe testuar) · **Data:** 2026-10-29

## Konteksti

Me 8 shard-e, çdo urdhër duhet të zgjedhë ku ekzekutohet. Nëse çdo urdhër prek të 8 shard-et, transaksionet bllokojnë të gjitha llogaritë e tregut dhe paralelizmi i matur në A-10 (~2,5× swap-e për slot) humbet. Nëse çdo urdhër shkon në një shard të vetëm, urdhrat e mëdhenj paguajnë më shumë rrëshqitje çmimi. Përdoruesi vendosi (29 tetor 2026): urdhrat e vegjël në 1 shard, të mëdhenjtë të ndarë në pak shard-e.

## Vendimi

Crate i ri `crates/router` (`amm-router`), vetëm off-chain. Programi onchain nuk ndryshon: plani dërgohet si një transaksion me një `swap` për çdo shard.

- **Libra (`Book`).** Gjendja e shard-it dhe tick arrays të lexuara nga zinxhiri. Kuota përdor të njëjtin motor (`engine::swap_exact_in`) dhe të njëjtën tarifë (bazë ose dinamike, ADR-0005) si programi, prandaj është e saktë deri në bajt, jo një përafrim.
- **Renditja.** Çdo shard kuotohet me `amount / max_shards`. Shard-et renditen sipas daljes, me id si barazues.
- **Pragu.** Për `k = 1, 2, …` urdhri ndahet në mënyrë të barabartë mes `k` shard-eve më të mirë. `k` rritet vetëm nëse dalja rritet të paktën `min_gain_bps` (parazgjedhje 2 bps). Kufijtë: `max_shards` (parazgjedhje 8) dhe kufiri i 64 llogarive për transaksion (`docs/tx-budget.md`).
- **Rebalancimi (`find_rebalance`).** Gjen shard-in më të lirë dhe më të shtrenjtë, provon fuqitë e 2-shit si sasi hyrëse, pastaj bën kërkim ternar rreth më të mirës, në të dy drejtimet. Kthen planin vetëm nëse fitimi është `≥ min_profit`. `apply_rebalance` e përditëson librin si programi. E përdor bot-i rebalancer i protokollit, sepse vetëm çelësat e listuar mund ta thërrasin `rebalance_pair` (ADR-0006).

Router-i nuk ka privilegj dhe nuk është i detyrueshëm. Kushdo mund të ndërtojë transaksionin vetë.

## Alternativat e refuzuara

- **Router onchain.** Do të kërkonte të gjitha shard-et si llogari në çdo transaksion, pikërisht atë që sharding-u shmang.
- **Ndarje optimale (pjesë të pabarabarta).** Fiton pak mbi ndarjen e barabartë kur shard-et janë afër njëri-tjetrit, që është gjendja që `rebalance_pair` e mban. Mund të shtohet më vonë pa ndryshuar programin.
- **Gjithmonë 8 shard-e.** Humb paralelizmin (A-10) dhe kushton më shumë CU.

## Evidenca

- `amm-router` (4 teste host): urdhër i vogël në 1 shard dhe i madh në disa; numri i shard-eve rritet me madhësinë dhe bie kur pragu rritet; urdhri i vogël zgjedh shard-in më të lirë; `find_rebalance` mbyll diferencën.
- `eng13_router` (2 teste onchain, programi SBF i vërtetë): për 1e6 dhe 4e10 në të dy drejtimet, tregtari merr saktësisht daljen e planit dhe çdo shard përfundon byte-për-byte ku thonë librat e router-it. Urdhri i vogël përdor 1 shard, i madhi disa. `find_rebalance` pas një swap-i të madh në shard-in 5 gjen `hi = 5`, dhe rebalancer-i i listuar, pa token, merr saktësisht fitimin e parashikuar.

Ende pa bërë: simulim i kostos së tregtarit dhe i numrit mesatar të shard-eve për urdhër me router-in dhe rebalancer-in tonë.
