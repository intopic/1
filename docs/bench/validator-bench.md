# Benchmark-u në validator (A-10)

Data: 29 tetor 2026 (përsëritur me kapital total të barabartë të njëjtën ditë; versioni i parë i jepte çdo shard-i të njëjtin likuiditet, pra 8 shard-e kishin 8 herë më shumë kapital, dhe dha të njëjtin raport 2,5×). Statusi: **NORMATIVE** për matjen; numrat janë nga një validator lokal me një nyje, jo nga mainnet.

## Çfarë mat

I njëjti numër swap-esh të pavarur (256 tregtarë × 16 = 4 096 për çdo provë) dërgohet në një `solana-test-validator` (Agave 4.3.2) me programin real SBF. Swap-et shpërndahen mbi 1, 2, 4 ose 8 shard-e. **Kapitali total është i njëjtë në çdo konfigurim:** secili ka tregun e vet me të njëjtin likuiditet total (L = 10^18), të ndarë në mënyrë të barabartë mes shard-eve që përdor. Shard-et e tjera të atij tregu mbajnë vetëm minimumin e gatishmërisë (L = 10^9), që tregu të hapet, dhe nuk marrin swap-e. Çdo tregtar ka payer-in dhe llogaritë e veta të tokenave, prandaj të vetmet llogari writable që dy swap-e mund të ndajnë janë ato të shard-it: `ShardState`, dy vault-et dhe tick arrays.

Kontrolli negativ: 8 shard-e, por i njëjti fee payer për çdo swap. Payer-i është writable në çdo transaksion, njësoj si një vault i përbashkët, prandaj swap-et konkurrojnë përsëri për një llogari.

Metrika kryesore është **swap/slot maks.**: sa swap-e të suksesshme hynë në slot-in më të ngarkuar. Për atë slot lexohet blloku dhe mblidhen CU-të që swap-et përdorën realisht.

## Rezultati (lokal, 4 CPU, 3 përsëritje, rendi i alternuar 1→8 dhe 8→1)

| konfigurimi | swap/slot maks. (mediana) | herë më shumë se 1 shard |
|---|---:|---:|
| 1 shard | 899 | 1.00 |
| 2 shard-e | 1782 | 1.98 |
| 4 shard-e | 2246 | 2.50 |
| 8 shard-e | 2247 | 2.50 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 884 | 0.98 |

| konfigurimi | tentativa | të dërguara | të suksesshme | dështuara | pa status | swap/slot maks. | CU në slot-in maks. (M) | vonesa p50 / p95 (slot) | dërgimi (ms) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 shard | 1 | 4096 | 4096 | 0 | 0 | 899 | 37.0 | 3 / 5 | 384 |
| 1 shard | 2 | 4096 | 4096 | 0 | 0 | 899 | 37.0 | 3 / 5 | 281 |
| 1 shard | 3 | 4096 | 3692 | 0 | 404 | 898 | 37.0 | 3 / 5 | 362 |
| 2 shard-e | 1 | 4096 | 4096 | 0 | 0 | 1786 | 74.0 | 2 / 3 | 504 |
| 2 shard-e | 2 | 4096 | 4032 | 0 | 64 | 1782 | 73.3 | 2 / 3 | 276 |
| 2 shard-e | 3 | 4096 | 4096 | 0 | 0 | 1738 | 71.7 | 2 / 3 | 545 |
| 4 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2231 | 92.6 | 2 / 3 | 469 |
| 4 shard-e | 2 | 4096 | 4096 | 0 | 0 | 2250 | 92.5 | 2 / 3 | 472 |
| 4 shard-e | 3 | 4096 | 3740 | 0 | 356 | 2246 | 92.5 | 2 / 3 | 284 |
| 8 shard-e | 1 | 4096 | 4096 | 0 | 0 | 2240 | 92.5 | 2 / 3 | 540 |
| 8 shard-e | 2 | 4096 | 3917 | 0 | 179 | 2248 | 92.5 | 2 / 3 | 301 |
| 8 shard-e | 3 | 4096 | 4096 | 0 | 0 | 2247 | 92.5 | 2 / 3 | 518 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 1 | 4096 | 4096 | 0 | 0 | 885 | 36.4 | 4 / 6 | 518 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 2 | 4096 | 3829 | 0 | 267 | 884 | 36.4 | 3 / 5 | 255 |
| 8 shard-e, payer i përbashkët (kontroll negativ) | 3 | 4096 | 4096 | 0 | 0 | 884 | 36.4 | 4 / 6 | 355 |

Asnjë swap nuk dështoi. "Pa status" janë transaksione që validatori i hodhi pa i ekzekutuar (`maxRetries: 0`, pa ridërgim); ato nuk numërohen si sukses. Tarifa e çdo transaksioni është tarifa bazë 5 000 lamports, pa priority fee. Swap-i kërkon 60 000 CU dhe përdor rreth 40 000.

## Si shpjegohet

Agave 4.3.2 kufizon CU-të për bllok dhe CU-të për çdo llogari writable brenda bllokut ([`block_cost_limits.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/cost-model/src/block_cost_limits.rs), [`slot_params.rs`](https://github.com/anza-xyz/agave/blob/v4.3.2/runtime/src/slot_params.rs)): 60M për bllok dhe 24M për llogari, ose 100M dhe 40M kur është aktiv feature-i `raise_block_limits_to_100m` (SIMD-0286). Test validator-i aktivizon të gjitha feature-t, prandaj këtu vlejnë 100M dhe 40M.

- **1 shard:** rreth 37M CU në slot, pra pishina godet kufirin për llogari. Një pishinë me një gjendje të vetme nuk mund të përdorë më shumë se 40% të bllokut, sado bosh të jetë blloku.
- **2 shard-e:** rreth 2 herë më shumë, sepse çdo shard ka kufirin e vet.
- **4 dhe 8 shard-e:** rreth 2.5 herë, 92M CU në slot. Këtu kufiri bëhet blloku, jo pishina. 4 dhe 8 shard-e japin të njëjtin numër, sepse blloku është plot.
- **Payer i përbashkët:** bie përsëri në nivelin e 1 shard-i. Ndarja ndihmon vetëm kur payer-at dhe llogaritë e tokenave të tregtarëve janë të ndara.

Raporti 2.5× nuk varet nga kufiri që është aktiv, sepse kufiri për llogari është 40% e bllokut në të dyja rastet (24/60 dhe 40/100).

## Çfarë nuk provon

- Nuk është mainnet: s'ka gossip, rotacion leader-ash, priority fees, MEV ose trafik nga programe të tjera. Në mainnet blloku ndahet me gjithë rrjetin, prandaj kufiri i bllokut arrihet rrallë nga një pishinë e vetme. Avantazhi i matur këtu është se pishina nuk ndalet te 40% e bllokut.
- Nuk mat kohën e ekzekutimit paralel në thread-e. Mat sa swap-e pranon blloku, që varet nga kufijtë e kostos.
- Nuk mat swap-e të ndara në disa shard-e në një transaksion (shih [tx-budget.md](../tx-budget.md)) ose cilësinë e çmimit (ENG-09).
- Me kapital të njëjtë, çdo shard ka 1/n e thellësisë. Swap-et e vegjël këtu nuk e matin slippage-in, që rritet në një shard të vetëm (shih ENG-09 dhe router-in, ADR-0007).
- Nuk justifikon fjalët "100% paralel" ose "zero bllokim".

## Si riprodhohet

```
bash scripts/build-linux.sh            # ndërton target/deploy/sharded_amm.so
bash scripts/bench-validator.sh 16 3   # shkruan target/bench/validator-bench.md
```

Në GitHub: workflow-i `bench` (`.github/workflows/bench.yml`, `workflow_dispatch` me `rounds` dhe `repeats`). Kodi: `tests/svm/examples/bench_setup.rs` (ndërton tregun në LiteSVM dhe i shkruan llogaritë për `--account-dir`) dhe `tests/svm/examples/bench_run.rs` (dërgon swap-et dhe mat).
