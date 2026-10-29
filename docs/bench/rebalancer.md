# Bot-i i rebalancimit kundër validatorit lokal

Bot-i jashtë zinxhirit për `rebalance_pair` (ADR-0006): [`tests/svm/examples/rebalancer.rs`](../../tests/svm/examples/rebalancer.rs), planifikimi në `amm_router::rebalance_round`. Programi onchain nuk ndryshon. Ekzekutohet nga [`scripts/rebalancer-validator.sh`](../../scripts/rebalancer-validator.sh) dhe workflow-i `rebalancer` në CI.

## Si punon

Çdo raund:

1. **Snapshot.** Tregu, 8 shard-et dhe tick arrays rreth çmimit të secilit lexohen me një `getMultipleAccounts`, pra nga i njëjti slot.
2. **Plani.** `rebalance_round` gjen deri në 4 rebalancime në çifte shard-esh që nuk ndajnë asnjë shard: hendekun më të gjerë, pastaj më të gjerin mes shard-eve që mbetën, e kështu me radhë. Çdo shard ka state, vault-e dhe tick arrays të vetat, kështu që transaksionet nuk shkruajnë asnjë llogari të përbashkët dhe mund të zbresin në të njëjtin slot.
3. **Pragu i fitimit.** Një plan dërgohet vetëm nëse fitimi në tokenin e vet është të paktën sa kostoja e një transaksioni në atë token (`--min-a`, `--min-b`). Onchain `min_profit` vendoset 90% e fitimit të parashikuar, por kurrë nën pragun. Nëse çmimet lëvizin ndërkohë, transaksioni dështon në vend që të paguajë më pak se kostoja.
4. **Dërgimi.** Plani `k` nënshkruhet dhe paguhet nga çelësi `k`, një nga 4 rebalancer-at e listuar. Asnjë payer i përbashkët, që do t'i serializonte (shih kontrollin negativ në [validator-bench.md](validator-bench.md)).
5. **Pas dërgimit.** `recovery::after_send`: dështimi dekodohet në gabimin e programit dhe klasifikohet. Transaksioni që nuk shihet ridërgohet me të njëjtat bajte sa kohë blockhash-i është i vlefshëm, dhe braktiset vetëm kur ka skaduar dhe historiku i plotë tregon që nuk ka ekzekutuar.

Ndryshime në router: `find_rebalance_min` ka prag të veçantë për fitimin në A dhe në B. Kur të dy drejtimet e kalojnë pragun, zgjidhet ai me vlerë më të madhe (A vlerësohet në B me mesataren gjeometrike të dy çmimeve). Më parë krahasoheshin numrat bruto të dy tokenëve. `RebalanceUnprofitable` dhe `RebalanceNoPriceGap` tani klasifikohen `Refresh`: për një plan që paguante në snapshot, kjo do të thotë vetëm që çmimet lëvizën.

Pragjet në test janë në njësi të tokenëve të testit. Në një treg real vendosen nga tarifa që paguan bot-i (5 000 lamports për nënshkrim plus priority fee) shumëzuar me çmimin e tokenit në lamports, plus një marzh.

## Testi

Tregu me 8 shard-e nga `bench_setup`, me 4 çelësa rebalancer të listuar (çdo çelës paguan tarifat e veta dhe nuk ka asnjë token), tarifë 0,30%.

- **Faza e qetë.** Tetë trader-a shtyjnë shard-et: çiftet poshtë, tekët lart, me sasi të ndryshme. Pastaj vepron vetëm bot-i. Kërkohet: çdo rebalancim zbret, të paktën një raund përdor dy ose më shumë çelësa, dhe hendeku bie nën brezin e tarifave + 5 bps.
- **Faza me zhurmë.** Trader-at bëjnë swap-e në shard-e të rastit ndërsa bot-i punon, kështu që disa snapshot-e vjetrohen. Kërkohet: çdo dështim klasifikohet `Refresh` (asnjë gabim tjetër), balancat e rebalancer-ave nuk zbresin kurrë, dhe pasi ndalet zhurma hendeku kthehet nën brez.

**Brezi i tarifave.** Një rebalancim paguan vetëm kur hendeku është më i gjerë se dy tarifat (2 × 30 = 60 bps). Brenda brezit asnjë rebalancim nuk ka fitim, prandaj hendeku mbetet rreth 60 bps. Kjo nuk është defekt: është kufiri ekonomik. Me tarifë 0,05% brezi do të ishte 10 bps.

## Rezultati (lokal, para commit-it)

Ekzekutim lokal me 200 swap-e zhurme, Agave 4.3.2, nga pema e punës para commit-it. Tabela me provenancë nga CI zëvendëson këtë sapo të kalojë workflow-i.

**Provenance:** commit `2580214e78cac8ef0be92b854c05c0ccf14abf14` (dirty), `sharded_amm.so` sha256 `e52cf1212bb983c048680f12da4dba2fda223be1715c6c6b86f10bc3fb68f651`, solana-test-validator 4.3.2 (src:a2033000; feat:c9ad34d2, client:Agave), 4 CPU.

Floors: 5000 A, 5000 B per transaction. Four rebalancer keys, each its own fee payer. Noise: 200 swaps.

| faza | raundi | hendeku para (bps) | pas (bps) | dërguar | zbritën | dështuan | ridërguar | skaduan | slot-e | maks. në 1 slot | fitimi A | fitimi B | CU maks./tx | tarifa (lamports) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| calm | 1 | 179.8 | 62.0 | 4 | 4 | 0 | 0 | 0 | 1 | 4 | 3359226146 | 1025896603 | 74016 | 20000 |
| calm | 2 | 62.0 | 60.3 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 0 | 540774 | 73072 | 10000 |
| noisy | 3 | 74.1 | 77.5 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 0 | 31173465 | 72975 | 10000 |
| noisy | 4 | 77.5 | 57.8 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 458851 | 47819342 | 73902 | 10000 |
| noisy | 5 | 73.9 | 70.1 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 29909530 | 73471 | 5000 |
| noisy | 6 | 70.1 | 82.8 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 0 | 1255946 | 73344 | 10000 |
| noisy | 7 | 82.8 | 85.2 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 165027310 | 58094914 | 73960 | 10000 |
| noisy | 8 | 85.2 | 77.7 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 0 | 100826741 | 73944 | 10000 |
| noisy | 9 | 77.7 | 94.4 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 49230299 | 73544 | 5000 |
| noisy | 10 | 94.4 | 95.9 | 3 | 2 | 1 | 0 | 0 | 1 | 2 | 0 | 49112459 | 73224 | 15000 |
| noisy | 11 | 95.9 | 74.1 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 204576183 | 72949 | 5000 |
| noisy | 12 | 74.1 | 63.5 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 28387587 | 0 | 72627 | 5000 |
| noisy | 13 | 63.5 | 63.3 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 1670463 | 72991 | 5000 |
| noisy | 14 | 63.3 | 109.7 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 1330080 | 0 | 72801 | 5000 |
| noisy | 15 | 109.7 | 77.6 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 393539022 | 74513 | 5000 |
| noisy | 16 | 77.6 | 82.5 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 109596775 | 0 | 72302 | 10000 |
| noisy | 17 | 82.5 | 78.1 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 96841278 | 0 | 73646 | 10000 |
| noisy | 18 | 78.1 | 93.1 | 2 | 1 | 1 | 0 | 0 | 1 | 1 | 14527109 | 0 | 72961 | 10000 |
| noisy | 19 | 93.1 | 90.1 | 2 | 2 | 0 | 0 | 0 | 1 | 2 | 0 | 238334743 | 73264 | 10000 |
| noisy | 20 | 90.1 | 109.1 | 1 | 0 | 1 | 0 | 0 | 1 | 0 | 0 | 0 | 65168 | 5000 |
| noisy | 21 | 109.1 | 96.9 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 459256350 | 73796 | 5000 |
| noisy | 22 | 96.9 | 72.3 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 322850640 | 0 | 72882 | 5000 |
| noisy | 23 | 72.3 | 56.6 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 23528484 | 73528 | 5000 |
| settle | 24 | 65.5 | 60.3 | 1 | 1 | 0 | 0 | 0 | 1 | 1 | 4021455 | 0 | 72364 | 5000 |

| dështimi → klasa | herë |
|---|---:|
| RebalanceUnprofitable → Refresh | 7 |

Hendeku: 179.8 bps pas shtyrjes, 60.3 bps pas fazës së qetë, 60.3 bps në fund. Brezi i tarifave (dy tarifa) është 60.0 bps: brenda tij asnjë rebalancim nuk paguan.

**Lexo kështu.** Në raundin e parë 4 rebalancime në 4 çifte të ndara zbritën në **të njëjtin slot**. Gjatë zhurmës, dështimet (`RebalanceUnprofitable`) ndodhin kur një swap i trader-it zbret para rebalancimit: transaksioni dështon, bot-i humb vetëm tarifën e transaksionit (5 000 lamports), jo tokena. CU për rebalancim: rreth 73k; me limit 300k.

## Kufijtë

Validator me një nyje në një makinë: pa gossip, pa rotacion lideri, pa bot-e konkurrente, pa treg priority fee. Nuk mat fitimin në mainnet. Kushdo mund ta mbyllë ende hendekun me dy swap-e të zakonshëm dhe kapitalin e vet (ADR-0006), kështu që në mainnet bot-i ka konkurrencë.
