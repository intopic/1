# Krahasimi me Raydium dhe Orca, vetëm me fakte të matura ose të verifikuara

Data: 29 tetor 2026. Statusi: **NORMATIVE** për çdo material publik. Çdo pretendim për AMM-në tonë lidhet me një test ose matje në këtë repo; çdo pretendim për Raydium dhe Orca lidhet me kodin e tyre publik në një commit të fiksuar. Kodi i Orca-s është lexuar vetëm për citim; asgjë nuk është kopjuar (ADR-0003).

Ky dokument zëvendëson tabelën e mëparshme të krahasimit. Fjalët "100% paralel", "zero bllokim", "MEV ekskluzive në Fly.io" dhe "56 shard-e në 1 transaksion" janë hequr, sepse asnjëra nuk ka matje që e mbështet, dhe dy prej tyre janë të pamundura sipas matjeve tona (shih më poshtë).

## Tabela

| Tema | Raydium CLMM | Orca Whirlpools | AMM-ja jonë (8 shard-e) |
|---|---|---|---|
| **1. Llogaritë që shkruan një swap** | `pool_state` është writable në çdo swap [R1] | `whirlpool` është writable në çdo swap, edhe `oracle` [O1] | `market` dhe `registry` vetëm për lexim; writable janë `ShardState`, dy vault-et dhe tick arrays të një shard-i, plus llogaritë e vetë tregtarit [S1] |
| **2. Sa swap-e të një tregu hyjnë në një bllok** | Nuk e kemi matur. Me një llogari writable për pishinë, kufiri i Agave për llogari (40% e bllokut) vlen për gjithë pishinën [S2] | Nuk e kemi matur. I njëjti kufi, për të njëjtën arsye [S2] | Matur në validator lokal me një nyje, me kapital total të barabartë: 899 swap/slot me 1 shard, **~2,5×** me 4 ose 8 shard-e (blloku plot, ~92M CU). Payer i përbashkët e kthen në 0,98× [S2] |
| **3. Trafik ekstrem** | Nuk e kemi matur | Nuk e kemi matur | Shard-et e ndarë nuk konkurrojnë për llogari, por blloku dhe kufiri për llogari mbeten. Swap-et në të njëjtin shard presin njëri-tjetrin si në çdo AMM. Në mainnet blloku ndahet me gjithë rrjetin; s'kemi matje atje [S2] |
| **4. Kush e mbyll diferencën e çmimit** | Kushdo me swap të zakonshëm; diferenca ndaj tregjeve të tjera (LVR) mbetet | Po ashtu | Mes shard-eve: `rebalance_pair` pa kapital, vetëm për çelësat rebalancer të protokollit (ndryshim me timelock 24h). LP-të marrin tarifat e të dy hapave. Kushdo tjetër mund të arbitrojë ende me swap-e të zakonshëm dhe kapitalin e vet, dhe ndaj tregjeve të jashtme LVR mbetet si te të tjerët. Nuk është MEV ekskluziv [S3] |
| **5. Tarifa** | Ka tarifë dinamike opsionale, me volatility accumulator (`dynamic_fee_config`) [R2] | Ka adaptive fee me `oracle` dhe `adaptive_fee_tier` [O1, O2] | Tarifë dinamike opsionale **për çdo shard**, 0,03%–2%, parametrat vetëm me timelock 24h; fikur si parazgjedhje. Simuluar, por jo e matur në treg real [S4] |
| **6. Transaksioni dhe kufiri i llogarive** | Mund të thirret nga transaksione v0 me ALT, si çdo program | Po ashtu | Kufiri real është **64 llogari të kyçura** për transaksion, edhe me ALT. Me v0 + ALT, një swap i ndarë hyn deri në **8 shard-e** (8 × 3 tick arrays + ComputeBudget = saktësisht 64). 56 shard-e në një transaksion do të kërkonin ≥ 287 llogari, prandaj është i pamundur [S5] |
| **7. Ndarja e urdhrave** | Një pishinë për çdo nivel tarife; agreguesit ndajnë mes pishinave | Po ashtu | Router off-chain me prag: urdhri i vogël në 1 shard, i madhi në disa, vetëm nëse çdo shard shtesë rrit daljen ≥ 2 bps. Dalja e planit = dalja onchain, deri në bajt [S6] |

## Burimet

Raydium dhe Orca (kod publik, commit i fiksuar):

- [R1] Raydium CLMM, `programs/amm/src/instructions/swap.rs`, `#[account(mut)] pool_state` — [ed7c84a5](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a5/programs/amm/src/instructions/swap.rs)
- [R2] Raydium CLMM, `programs/amm/src/states/dynamic_fee_config.rs` dhe `dynamic_fee_info` në swap — [6d7a0f82](https://github.com/raydium-io/raydium-clmm/tree/6d7a0f82d2f293c553c69aaf943377367c6c4cc6/programs/amm/src/states)
- [O1] Orca Whirlpools, `programs/whirlpool/src/instructions/v2/swap.rs`: `#[account(mut)] whirlpool`, `#[account(mut)] oracle`, `get_adaptive_fee_info` — [cc683ec9](https://github.com/orca-so/whirlpools/blob/cc683ec92acb5e7606caa3d8e514d9383e9c7c11/programs/whirlpool/src/instructions/v2/swap.rs)
- [O2] Orca Whirlpools, `programs/whirlpool/src/state/adaptive_fee_tier.rs` — [cc683ec9](https://github.com/orca-so/whirlpools/tree/cc683ec92acb5e7606caa3d8e514d9383e9c7c11/programs/whirlpool/src/state)

Këtu (repo-ja jonë):

- [S1] `programs/sharded-amm/src/instructions/swap.rs`; [account-layout.md](account-layout.md)
- [S2] [bench/validator-bench.md](bench/validator-bench.md) (A-10), me kufijtë e Agave 4.3.2 `block_cost_limits.rs`
- [S3] [ADR-0006](adr/0006-open-rebalance-pair.md), `tests/svm/tests/eng12_rebalance.rs`
- [S4] [ADR-0005](adr/0005-dynamic-fee-per-shard.md), [sim/dynamic-fee-results.md](sim/dynamic-fee-results.md)
- [S5] [tx-budget.md](tx-budget.md), `tests/svm/tests/tx_budget.rs`
- [S6] [ADR-0007](adr/0007-threshold-router.md), `tests/svm/tests/eng13_router.rs`

## Pretendimi i vetëm publik i lejuar sot

> Në një validator lokal me një nyje, me kapital total të barabartë, një treg me 8 shard-e pranoi rreth 2,3–2,5 herë më shumë swap-e për slot se i njëjti treg me 1 shard, kur urdhrat e vegjël shpërndahen mbi shard-e (me router-in tonë: 2,30× në GitHub Actions, 2,43× në matjen e parë lokale).

Çdo pretendim tjetër për performancë kërkon një matje të re, të shtuar në këtë dokument me burimin e vet.

## Korrigjime ndaj tabelës së mëparshme

- "Orca dhe Raydium kanë tarifë statike": e pasaktë. Të dy kanë tarifë dinamike/adaptive opsionale [R2, O1, O2]. Dallimi ynë është vetëm se gjendja e tarifës është për shard, jo për pishinë.
- "Raydium/Orca kufizohen nga 32 llogari dhe kërkojnë ALT": transaksionet v0 me ALT vlejnë për çdo program, edhe për tonin. Kufiri i 64 llogarive të kyçura vlen për të gjithë [S5].
- "Llogaria qendrore nuk bllokohet kurrë" dhe "zero bllokim": llogaritë e shard-it bllokohen si çdo pishinë; ndarja vetëm i shpërndan ato në 8 [S1, S2].
