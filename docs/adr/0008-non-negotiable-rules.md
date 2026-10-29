# ADR-0008 — Tetë rregullat e panegociueshme

**Statusi:** ACCEPTED · **Data:** 2026-10-29 · **Burimi:** vendimi i përdoruesit, 29 tetor 2026

## Konteksti

Avantazhi kryesor i arkitekturës është grafiku i llogarive writable. Në swap-in e Orca-s (`whirlpool` writable, [cc683ec9](https://github.com/orca-so/whirlpools/blob/cc683ec92acb5e7606caa3d8e514d9383e9c7c11/programs/whirlpool/src/instructions/swap.rs)) të gjitha tregtitë e një pool-i shkruajnë të njëjtën gjendje. Te ne, dy tregti në shard-e të ndryshme shkruajnë gjendje dhe vault-e të veçanta. Kjo krijon mundësi ekzekutimi paralel vetëm nëse nuk konfliktojnë as llogaritë e tjera të transaksionit (payer-i, llogaritë e tokenave).

Kjo nuk provon vetvetiu 8× throughput, zero konflikte ose fitim më të madh. Ndarja e të njëjtit kapital në 8 pjesë e rrit slippage-in në një shard të vetëm.

## Vendimi: rregullat dhe ku zbatohen

| # | Rregulli | Zbatimi | Evidenca |
|---|---|---|---|
| 1 | Çdo shard mbulon detyrimet e veta: principal sipas pagesës reale, tarifa LP të regjistruara dhe të fituara ende pa checkpoint, tarifa protokolli | Vault-e dhe kontabilitet për shard; rrumbullakim në favor të pool-it | `engine_model` (aftësi paguese ekzakte pas çdo veprimi), eng06 dhe stresi: të gjithë LP-të dalin plotësisht |
| 2 | Swap-i nuk shkruan gjendje të përbashkët | `market` dhe `registry` vetëm për lexim; tarifa dinamike dhe numëruesit në `ShardState` | `instructions/swap.rs`, [account-layout.md](../account-layout.md), A-10 |
| 3 | Hapja vetëm me 8 shard-e të verifikuara, `active_mask = 0xff` | `finalize_initial` kërkon `prepared_mask` dhe `ready_mask == 0xff` | eng04 |
| 4 | Aktivizimi kërkon kapital dhe thellësi të përdorshme në të dy drejtimet | **I ri:** `verify_depth`. Dy swap-e provë mbi kopje: shitja e `depth_probe_a` A dhe e `depth_probe_b` B duhet të mbushen plotësisht brenda `max_depth_impact_bps` tick-ësh (1 tick ≈ 1 bp). Shënjohet `ShardState.depth_verified`, dhe çdo `decrease_liquidity` e fshin. `open_market` dhe `activate_shard` e kërkojnë. Tick arrays jepen vetëm për lexim | `engine::depth` (4 teste host), `eng14_depth` (4 teste onchain) |
| 5 | Zgjerimi nuk transferon fshehurazi claims | Shard-i i ri financohet me kapital të ri; pozicionet lidhen me shard-in (PDA) | eng07 |
| 6 | Maska regjistron anëtarësinë; s'përdoret `hash & 55` | `select_by_hash` zgjedh ndër bit-et aktive | `math::bitmap` (përfshirë testin që tregon pse `hash & 55` është i gabuar) |
| 7 | Keeper-i ka pushtet të kufizuar | Nuk lëviz fonde; e vendos çmimin vetëm një herë; aktivizimi kontrollon devijimin nga referenca; rebalancimi ka kufij çmimi | eng03, eng04, eng07, eng12 |
| 8 | Çdo pretendim performance matet me kapital total të barabartë | Benchmark-u A-10 u përsërit: çdo konfigurim ka tregun e vet me të njëjtin L total, të ndarë mes shard-eve që përdor | [validator-bench.md](../bench/validator-bench.md): sërish 2,5×; ENG-09 përdor $1M për çdo dizajn |

## Pasojat

- **Rregulli 4:** keeper-i (ose kushdo) duhet të thërrasë `verify_depth` pas depozitave dhe para `open_market` ose `activate_shard`. Nëse një LP tërheq likuiditet nga një shard i përgatitur, prova duhet përsëritur. Shkruhet vetëm flamuri i shard-it. Për sa i takon numrit të llogarive, prova bëhet në instruksionin e vet, jo brenda `open_market`.
- **Kufiri:** prova mat thellësinë në atë çast. Pas aktivizimit LP-të mund të dalin, sepse daljet nuk bllokohen kurrë (ADR-0002). Router-i duhet ta kuotojë çdo herë.
- **Ende pa bërë:** router-i (ADR-0007) zgjedh vetëm sipas çmimit neto, jo sipas ngarkesës së shard-it. Kosto e ndarjes së urdhrave (llogari, CU, shard-e të kyçura) matet në [tx-budget.md](../tx-budget.md), por jo nën ngarkesë reale.
