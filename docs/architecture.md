# Arkitektura e propozuar — nisje me 8 shard-e dhe zgjerim deri në 56

> **Dokument historik (para implementimit).** Ruhet si kontekst. Kur ndryshon nga kodi ose nga dokumentet normative, vlejnë [numeric-spec.md](numeric-spec.md), [account-layout.md](account-layout.md), [tx-budget.md](tx-budget.md), [ADR-të](adr/README.md) dhe [STATUS.md](../STATUS.md). Indeksi: [docs/README.md](README.md).

Status: specifikim para implementimit, 28 tetor 2026. Ky dokument përshkruan një objektiv për një program të ardhshëm Solana. Modeli referencë dhe simulimet në këtë workspace nuk janë kontratë e deploy-uar dhe nuk provojnë paralelizëm në validator, siguri CPI apo mbulim të plotë të CLMM.

Kodi aktual në `crates/amm-model` është vetëm kernel kërkimor Rust me rezerva virtuale constant-product: `Band::new`, `quote_exact_in`, `swap_exact_in`, dy drejtime, tarifë e rrumbullakosur lart dhe output poshtë me aritmetikë të kontrolluar. Nuk ka kufij intervali, tick crossing, registry 8→56 ose gjendje shard-esh, vault-e, pozicione apo likuidim. Specifikimi më poshtë nuk pretendon se këto janë ndërtuar.

## Vendimi kryesor

MVP i propozuar do të nisë me 8 shard-e aktive me ID `0..7`, me kapacitet deri në 56 ID `0..55`. Secili është një treg likuiditeti i pavarur për të njëjtin çift tokenësh, me rezervat, çmimin, likuiditetin aktiv, tick-et dhe tarifat e veta. Router-i zgjedh shard-in ose një grup të kufizuar shard-esh jashtë programit; programi verifikon kufijtë e nënshkruar nga përdoruesi. Nuk premtohet një çmim global i vetëm apo mungesë absolute konfliktesh.

Ndarja vetëm e tick-eve nuk mjafton nëse swap-et vazhdojnë të shkruajnë një `PoolState`, vault apo akumulator tarifash të përbashkët. Shard-et me çmime të pavarura kanë fragmentim likuiditeti dhe mundësi arbitrazhi; këto janë pjesë e modelit ekonomik që duhet matur.

## Komponentët

| Komponenti | Përgjegjësia | Shkrime në rrugën e swap-it |
|---|---|---|
| `MarketConfig` | Identiteti i tregut, mint-et, programet token, kufijtë e parametrave dhe versioni i skemës | Asnjë; lexohet |
| `ShardRegistry` | `active_mask:u64`, `prepared_mask:u64`, `registry_version:u64`; lidhja me market | Vetëm lexim; shkrim në operacione kontrolli |
| `ShardState` për çdo ID të përgatitur, maksimumi 56 | Gjenerata/statusi, çmimi lokal, likuiditeti aktiv, rezervat e kontabilizuara dhe rritja e tarifave | Vetëm shard-i i përdorur |
| `VaultA/B(shard)` | Tokenët realë të shard-it, me autoritet PDA të shard-it | Vetëm vault-et e përdorura |
| `TickArray(shard,index)` | Tick-e të CLMM dhe kontabiliteti në kufijtë e tyre | Vetëm vargu i deklaruar që preket |
| `LpPosition(owner,shard,id)` | Likuiditeti, intervali dhe checkpoint-et e tarifave të pozicionit | Në shtim/heqje likuiditeti; jo në çdo swap |
| `ReserveLane` lokale, fazë e mëvonshme | Indeks borxhi, kuotë huadhënieje, borxh dhe humbje lokale | Vetëm operacionet e huasë/likuidimit |
| `DebtPosition(owner,lane,id)` | Detyrimi dhe kolaterali i përdoruesit | Vetëm pozicioni që ndryshon |
| Oracle i jashtëm | Çmimi për vlerësimin e kolateralit, me feed identity dhe kufij freskie/besueshmërie | Lexim gjatë likuidimit |

LP-të përdorin pozicione PDA, pa mint global fungible LP në rrugën e nxehtë. Tokenët mint A/B normalisht lexohen; veçoritë Token-2022 që kërkojnë llogari shtesë ose shkrime trajtohen si zgjerim i ardhshëm me analizë të veçantë. MVP i ardhshëm duhet të kufizojë programet/extension-et e mbështetura shprehimisht.

## Nisja me 8 dhe rritja deri në 56

`INITIAL_MASK = 0xff_u64`; `VALID_MASK = (1_u64 << 56) - 1`. Bit-et `56..63` janë të rezervuara dhe duhet të mbeten zero në të dy maskat. `prepared_mask` ruan ID-të me llogari të inicializuara dhe identitete të verifikuara, përfshirë ato aktive; `active_mask` është nënset i saj. Prepared nuk është provë e financimit apo thellësisë aktuale: tërheqjet gjatë joaktivitetit mund ta ndryshojnë gjendjen. Çdo aktivizim/finalizim rivlerëson rezervat, likuiditetin dhe pretendimet LP. Maskat identifikojnë anëtarësinë, jo lock-et, threads apo buxhetin e llogarive të një transaksioni.

Nisja bëhet në hapa të kufizuar: përgatit `ShardRegistry`/config në gjendje tregu joaktiv; përgatit dhe financo secilin nga 8 shard-et veçmas; pastaj `finalize_initial_shards` kontrollon të tetë state/vault-et, mint-et, autoritetet dhe thellësinë minimale, dhe aktivizon atomikisht bit-et `0..7` me statuset përkatëse. Fondet fillestare kanë burim/LP pretendime të regjistruara; nuk janë rezervë pa pronar. Asnjë swap nuk pranohet para finalizimit.

Për shtim të mëvonshëm, keeper-i propozon një ID joaktive nën 56 sipas likuiditetit dhe ngarkesës së matur. Programi përgatit PDA/vault-et dhe regjistron financimin/pretendimet përkatëse pa e bërë shard-in të tregtueshëm. Aktivizimi verifikon fondet reale, identitetet, statusin dhe kufijtë e thellësisë; shkruan atomikisht bit-in e registry-t dhe `ShardState.status=Active`, dhe rrit versionin. Vetëm shtimi i një bit-i pa financim nuk lejohet.

`min_depth` dhe trigger-i i rritjes janë politikë e konfigurueshme që kërkon specifikim njësish, vlere dhe oracle-i. Kriteri duhet të masë thellësinë e kuotueshme në të dy drejtimet për trade të caktuar dhe slippage maksimal, inventory real dhe hapësirën deri te kufijtë/tick-et; TVL jashtë intervalit nuk provon likuiditet aktiv. Një prag në USD kërkon oracle, decimals dhe freshness; një prag në njësi mint është specifik për çiftin. Keeper-i vendos kur të propozojë; kontrata verifikon kushtet e koduara. Kontrata nuk zgjohet dhe nuk dërgon transaksione vetë. Përgatitja nuk garanton që tregu do të aktivizojë patjetër 56 shard-e; thellësia për shard mund ta bëjë zgjerimin të padobishëm.

Swap-et lexojnë `ShardRegistry` dhe shkruajnë vetëm shard-in/vault-et e tyre. Aktivizim/deaktivizim shkruan registry-n dhe mund të konfliktojë përkohësisht me lexuesit; registry nuk përditësohet për numërues apo zgjedhje route-i në çdo swap. Versioni global shërben për operacionet e kontrollit; siguria e swap-it kontrollon maskën aktuale dhe gjeneratën lokale, pa kërkuar domosdoshmërisht barazi me versionin global të një quote-i për një shard tjetër.

Router-i mund të zgjedhë ekonomikisht mes ID-ve aktive. Nëse përdor hash për shpërndarje: `rank = hash % popcount(active_mask)` dhe zgjidh bit-in aktiv me atë rend zero-based; maska bosh refuzohet. `hash & 7` vlen vetëm kur maska është saktësisht prefiksi fillestar `0xff`. `hash & 55` është gabim për 56 dhe nuk përdoret; aktivizime/deaktivizime mund të krijojnë boshllëqe. Hash-i është shpërndarje ngarkese, jo provë e çmimit më të mirë apo kufi sigurie; përdoruesi mund të kërkojë vlera hash të favorshme.

Një shard i financuar nga kapital i ri nuk është split i LP-ve ekzistuese. Marrja e kapitalit nga një shard prind kërkon migrim me ruajtje të pretendimeve, tarifave dhe rezervave, siç përshkruhet më poshtë. Rebalancimi kufizohet në çifte ose sete të vogla të deklaruara; kapaciteti 56 nuk kërkon prekjen e të 56-ve atomikisht.

## Swap dhe kufijtë e atomicitetit

1. Klienti lexon gjendjen dhe zgjedh shard-in, gjeneratën, tick arrays dhe kufijtë `amount_in`, `min_amount_out`, `sqrt_price_limit`, `deadline`.
2. Mesazhi deklaron paraprakisht të gjitha llogaritë, përfshirë ato për CPI token.
3. Programi kontrollon pronësinë, PDA-të, mint-et, autoritetet, statusin aktiv dhe gjeneratën përpara ndryshimit të rezervave.
4. Llogaritja lokale përditëson gjendjen/tarifat dhe kryen transfertat me CPI. Dështimi i një kontrolli rrëzon të gjithë operacionin sipas atomicitetit të transaksionit.

Një route me disa shard-e mban lock-et e të gjitha llogarive të deklaruara për atë transaksion. Instruksionet brenda transaksionit nuk bëhen paralelisht nga kjo arkitekturë. Përfitimi synohet midis transaksioneve që prekin shard-e të ndryshme.

Për dy transaksione me sete leximi `R1,R2` dhe shkrimi `W1,W2`, kushti i pavarësisë është `W1 ∩ (R2 ∪ W2) = ∅` dhe `W2 ∩ (R1 ∪ W1) = ∅`. Payer-i, ATA-të e bot-it, destinacioni i tarifave, vault-i flashloan apo një oracle i shkruar gjatë të njëjtit route mund ta prishin kushtin edhe kur shard-et ndryshojnë.

## Huadhënia dhe likuidimi

Huadhënia është modul i veçantë me reserve lanes dhe kuota lokale; swap-et nuk shkruajnë `ReserveLane`. Një lane ka vault borxhi, vault kolaterali, indeks interesi, borxh dhe buxhet risku lokal. Në modelin fillestar, një detyrim nuk shpërndahet ndërmjet lane-ve.

Një kufi global zbatohet nëpërmjet kuotave të rezervuara: `sum(cap_i) <= cap_global` dhe `used_i <= cap_i`. Përdorimi real global nuk rillogaritet në një llogari writable në çdo veprim. Transferimi i kuotës është operacion kontrolli që duhet të debitojë burimin dhe kreditojë destinacionin atomikisht ose me një capability njëpërdorimëshe të provuar; ky mekanizëm nuk konsiderohet i implementuar.

Likuidimi verifikon identitetin/freskinë e oracle-it dhe shëndetin e pozicionit, pranon shlyerjen e kufizuar dhe liron kolateralin me rregulla të qarta rounding/bonus. Nëse kërkohet swap, route-i shton setin e llogarive AMM. Një flashloan nga një vault global rikthen konfliktin mbi atë vault. MVP i modelit nuk provon ende një likuidim të ekzekutueshëm me token CPI.

## Split/merge dinamik — fazë e mëvonshme

MVP i propozuar nis me 8 shard-e; shtimi i shard-eve të reja të financuara lejon rritje deri në 56. Split/merge i pozicioneve ekzistuese mbetet jashtë MVP-së. Modeli i mundshëm është `Active(g) -> Frozen(g) -> Retired(g)` me fëmijë `Active(g+1)`, identitete të reja dhe vault-e të veçanta. Operacioni duhet të deklarojë të gjitha llogaritë e prindit/fëmijëve; derivimi i PDA-së nuk jep aftësi për të ngarkuar një llogari të padeklaruar gjatë ekzekutimit.

Kalimi duhet të ruajë rezervat, detyrimet LP, tarifat e fituara, rounding dhe kuotat. Route-et e vjetra refuzohen nga statusi/gjenerata e prindit. Një directory writable në çdo swap nuk lejohet. Directory e lexuar gjatë swap-it mund të përdoret për discovery, por përditësimi i saj ende konflikton përkohësisht me lexuesit.

Migrimi i shumë LP-ve nuk mund të supozohet se futet në një transaksion: duhet vendosur mes migrimit të kufizuar atomik dhe migrimit me kërkesa të verifikueshme, gjatë të cilit fondet për kërkesat e pambyllura mbeten të rezervuara. Mbyllja e vault-eve të vjetra nuk lejohet përpara shlyerjes së të gjitha kërkesave. Protokolli i saktë është pyetje e hapur, jo veçori e MVP-së.

## Transaction v1: çfarë ndryshon

v1 është aktiv në mainnet nga 15 shtatori 2026, epoch 1035. Kufiri është 4,096 bajt dhe 64 adresa totale inline; v1 nuk përdor ALTs. v0 me ALT arrinte tashmë të njëjtin kufi runtime prej 64 llogarish. Pra, v1 e lehtëson paketimin dhe discovery, por nuk sjell mekanizëm të ri për lock-et apo provë se sharding-u më parë ishte i pamundur.

64 është buxhet për të gjitha adresat e transaksionit, jo 64 shard-e. Payer, përdoruesi, programet, mint-et, oracle-t, state, vault-et dhe tick arrays e konsumojnë atë. Router-i duhet të verifikojë madhësinë reale të mesazhit dhe numrin e adresave pas deduplikimit. v1 duhet të ketë kufijtë compute/data të vendosur shprehimisht dhe wallet që deklaron mbështetje për versionin 1.

## Vendime të pambyllura para programit onchain

- CLMM i plotë kundrejt motorit fillestar me interval të vetëm; kalimet e tick-eve dhe kontabiliteti fee-growth kërkojnë provë të veçantë.
- Modeli i rishpërndarjes së likuiditetit, kostoja e arbitrazhit dhe kushtet kur 8–56 lane janë më mirë se një pool.
- Kufijtë e riskut, interesi, oracle-t, bonuset e likuidimit dhe trajtimi i borxhit të keq.
- Shkrimet e observimeve për TWAP: lokale për shard ose jashtë rrugës; asnjë çmim AMM lokal si oracle i vetëm për huanë.
- Politika e upgrade authority, administrimit dhe pauzës; ndryshimi i config konfliktet me swap-et që e lexojnë.
- Formati final i seed-eve, versionimi ABI dhe mbështetja wallet/SDK në mjedisin e testuar.

## Burime të verifikuara

- [Statusi zyrtar i Transaction v1 dhe kufijtë](https://solana.com/upgrades/larger-transaction-sizes).
- [SIMD-0385: formati](https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0385-transaction-v1.md); metadata e propozimit nuk zëvendëson statusin e aktivizimit.
- [Runtime dhe varësitë e deklaruara](https://beta.docs.solanalabs.com/validator/runtime), [pipeline dhe AccountInUse](https://solana.com/docs/core/transactions/transaction-pipeline).
- [Rregullat CPI](https://solana.com/docs/core/cpi), [warmup i ALT-ve të reja/zgjatura](https://github.com/solana-foundation/developer-content/blob/main/content/courses/program-optimization/lookup-tables.md).
- [Shembull konkret PoolState i Raydium](https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/pool.rs). Ky shembull nuk provon pretendimin për çdo AMM ose protokoll huadhënieje.


