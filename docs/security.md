# Modeli i kërcënimeve dhe kufijtë e sigurisë

Data: 28 tetor 2026. Statusi: specifikim kërkimor; nuk është auditim, kontratë e vendosur ose autorizim për kapital real.

## Fusha dhe supozimet

Objektivi aktual i MVP-së është tetë lane fillestare AMM, me specifikim zgjerimi dinamik deri në 56. Secili ka gjendjen, vault-et, tarifat dhe pozicionet LP të veta. Konfigurimi dhe regjistri i përbashkët lexohen gjatë swap-it. Tetë lane-et dhe bitmask-i ende nuk janë implementuar; testi historik E-01 nuk i provon. Huadhënia/likuidimi dhe migrimi i pretendimeve LP janë faza të mëvonshme. Modeli lokal matematikor nuk provon sjelljen e validatorit, CPI-të SPL Token, sigurinë e custody-t apo performancën e rrjetit.

Kundërshtari kontrollon transaksione, renditje ekonomike përmes tarifave/MEV, llogari të kaluara në instruksion, tokenë të dërguar drejtpërdrejt në vault dhe përditësime oracle të zgjedhura prej tij. Ai mund të përdorë flashloan, të përsërisë kërkesa, të konkurrojë me likuidime/migrime dhe të shkaktojë dështim në kufijtë e CU, bajteve e llogarive. Nuk supozojmë komprometim të konsensusit ose thyerje të kriptografisë; komprometimi i administratorit dhe upgrade authority mbetet rrezik operacional që duhet kufizuar.

Për transaksionet A dhe B, mungesa e konfliktit kërkon të dyja: `W_A ∩ (R_B ∪ W_B) = ∅` dhe `W_B ∩ (R_A ∪ W_A) = ∅`. Ndërvarësia përfshin edhe fee payer, llogaritë e përdoruesit dhe çdo CPI. Pohimi "zero-contention" vlen vetëm për bashkësi konkrete llogarish që e plotësojnë këtë kusht; nuk është garanci throughput-i.

## Asetet dhe invariantat e detyrueshme

- **Fonde:** daljet e autorizuara nuk tejkalojnë inventarin dhe pretendimet e vlefshme. Ndryshimet e vault-it duhet të përputhen me ndryshimet e kontabilitetit dhe transferimet faktike, në njësitë bazë të mint-it.
- **LP dhe tarifa:** tërheqja konsumon pretendimin përkatës vetëm një herë; tarifat e fituara nuk fshihen nga ndryshimi i likuiditetit. Rrumbullakimi dhe dust-i kanë rregull eksplicit, konservativ dhe të kufizuar.
- **Likuiditet:** një lane nuk përdor rezervat e lane-it tjetër pa transferim të autorizuar që ndryshon të dyja anët. Balanca e papritur nga një donacion nuk krijon automatikisht LP shares.
- **Borxh:** rritja dhe shlyerja lidhen me të njëjtin indeks/lane/gjeneratë. Borxhi i parekuperueshëm regjistrohet; likuidimi nuk garanton aftësi paguese gjatë çdo hendeku çmimi.
- **Kuota:** `used_i ≤ cap_i` dhe `Σ cap_i ≤ cap_global`. Një snapshot global readonly nuk autorizon dy herë të njëjtën kapacitet.
- **Migrim:** një pretendim ka një gjeneratë kanonike aktive. Shuma e inventarëve dhe detyrimeve të fëmijëve përputhet me burimin, me dust të deklaruar; LP liquidity nuk shtohet përsëri gjatë migrimit lazy.

## Regjistri i rreziqeve

| ID | Kërcënimi / ndikimi | Masa e kërkuar dhe prova |
|---|---|---|
| SEC-01 | Llogari e zëvendësuar, PDA e gabuar, mint ose program token i rremë; vjedhje fondesh | Verifiko owner, discriminator/version, seeds+bump, pool/lane/generation, authority, mint dhe token program. Refuzo alias-e të palejuara burim/destinacion. Teste negative për çdo lidhje. |
| SEC-02 | Gabim fixed-point, overflow, rrumbullakim në favor të sulmuesit | Aritmetikë checked, domain i kufizuar, referencë racionale e pavarur, drejtim konservativ rrumbullakimi dhe cikle swap/LP pa nxjerrje vlere nga dust-i. |
| SEC-03 | Vault balance dhe kontabilitet divergojnë | Krahaso delta reale para/pas CPI; trajto donacionet sipas politikës së deklaruar. Konservim për çdo mint dhe asnjë pretendim i dyfishtë. |
| SEC-04 | LP tërheq dy herë, humb tarifa ose përfiton nga shkelja e kufirit tick | Pozicion PDA individual, checkpoints tarifash dhe konsum i saktë i L; teste për kufij dhe rrugë add/remove/collect. Pa mint global LP në hot path. |
| SEC-05 | Shard-e që prekin të njëjtin vault/ATA/payer/treasury/flashloan source | Inventar i plotë R/W për mesazhin dhe CPI. Vault-e e fee accumulation lokale; payer dhe token accounts të ndryshme në benchmark paralel. Shkallëzimi nuk provohet me emra të ndryshëm PDA. |
| SEC-06 | Oracle i gabuar, i vjetër, me timestamp të ardhshëm, exponent/decimals të gabuar ose pasiguri të lartë | Verifikim programi dhe feed ID, njësi kohore eksplicite, kufij freskie/future skew/confidence, çmime pozitive dhe valuation konservativ. Testo përzgjedhjen e update-ve historike brenda pragut. Politika për outage duhet të dallojë borrow/withdraw nga repay. |
| SEC-07 | Likuidim mbi pozicion të shëndoshë, shpërblim i tepërt, humbje e fshehur | Kontroll shëndeti në ekzekutim; kufi shlyerjeje dhe kolaterali, bonus i përcaktuar, min-out/deadline, trajtim dust/borxh i mbetur; përfundim atomik. Testo konkurrencë repay/withdraw/liquidate. |
| SEC-08 | Dyfishim kuotash ose kapaciteti global nga punë paralele | Kuota lokale të konsumueshme. Rialokim atomik donor+marrës ose escrow me identitet/version që konsumohet një herë. Refuzim i witness të vjetër dhe replay. |
| SEC-09 | Sandwich, arbitrage nga gjendje stale dhe botë që favorizojnë një lane | Min-out/max-in, kufi çmimi dhe afat; quote nuk është garanci ekzekutimi. Simulime renditjeje dhe stres hotspot; mat slippage, adverse selection dhe humbje LP, jo vetëm TPS. |
| SEC-10 | Migrim me dy prindër aktivë, fëmijë të mangët ose mbyllje të hershme | State machine/version eksplicit; dëshmitarë old/new; aktivizim atomik dhe konsum i pretendimit. Testo ndërprerje/retry në çdo hap. Asnjë freeze pa rrugë rikuperimi dhe analizë të vonesës së likuidimit. |
| SEC-11 | Token-2022 transfer fee/hook/delegate ndryshon semantikën ose CPI-të | Allowlist e qartë programesh dhe extensions. MVP kufizohet te transferime të mbështetura; mbështetja shtesë kërkon delta reale dhe teste adversariale. |
| SEC-12 | Upgrade/admin i komprometuar, drain ose ndryshim i menjëhershëm rreziku | Autorizime minimale, gjurmë ndryshimesh, pause selektiv, procedurë upgrade/rollback dhe rishikim i pavarur përpara kapitalit real. Testo çdo instruksion të privilegjuar. |
| SEC-13 | CU/account/size limit, version v1 ose konfigurim i gabuar; dështim likuidimesh | Test validator real për mesazhe dhe CPI; limite CU dhe loaded-data të shprehura në config v1. Mos u mbështet te introspektimi i instruksioneve ComputeBudget për kufizim tarifash v1. Rrugë bounded dhe refuzim determinist. |
| SEC-14 | Bit i palejuar ose shift i pakontrolluar zgjedh shard të gabuar | Maskë `u64`, vetëm bit-et 0..55; kontroll `id < 56` përpara shift-it dhe përdor checked shift. Refuzo id 56..63 dhe çdo input më të madh; refuzo maskë me bit-e të sipërme. Count llogaritet me `count_ones()`, maksimumi 56. |
| SEC-15 | Registry dhe status/fondet divergojnë gjatë aktivizimit ose garës me swap | Aktivizimi në control plane shkruan registry dhe gjendjen përkatëse atomikisht, kontrollon gjeneratën/statusin dhe funded-ready; swap lexon registry readonly, shkruan state/vault-et lokale dhe verifikon se shard-i është aktiv e i lidhur me registry. Testo të dy renditjet seriale dhe rollback. |
| SEC-16 | Pool i pjesshëm bëhet aktiv, ose mblidhet rent për 56 shard-e të panevojshme | Përgatit 8 shard-et fillestare me hapa të versionuar e të riprovueshëm; finalizo vetëm pasi të tetë plotësojnë kërkesat. Maskë fillestare pas finalizimit `0xff`; shard-et 8..55 krijohen sipas nevojës. |
| SEC-17 | Aktivizim bosh, humbje e pretendimeve LP ose fonde të bllokuara pas çaktivizimit | Biti nuk krijon likuiditet. Aktivizimi kërkon vault-e të verifikuara, likuiditet të mjaftueshëm sipas domain-it të modelit, kontabilitet dhe alokim LP të përfunduar. Një shard joaktiv por me fonde ruan rrugë të sigurt tërheqjeje/collect për pretendimet e vlefshme. |
| SEC-18 | Routing në bit joaktiv ose shpërndarje e gabuar mbi maskë sparse | `hash & 55` është i pavlefshëm për indeksim të përgjithshëm. Zgjidh rank në `0..count_ones(mask)` dhe pastaj bit-in aktiv të atij rank-u; maska zero refuzohet. Validimi onchain i id/bit/statusit mbetet i detyrueshëm pavarësisht router-it. |

## Regjistri 8 → 56 dhe aktivizimi

`active_mask: u64` ka domain `active_mask & !((1u64 << 56) - 1) == 0`. Shard ID është pozicioni i bit-it, jo rendi i tij në listën e bit-eve aktive. Përpara `1u64.checked_shl(id)` zbatohet guard-i `id < 56`; fakti që u64 mund të mbajë bit-e deri në 63 nuk i bën ato ID të vlefshme. `active_count`, nëse ruhet veçmas, duhet të barazohet me `active_mask.count_ones()`; prefero derivimin për të shmangur dy burime të së vërtetës. Maska zero mund të ekzistojë gjatë përgatitjes ose pauzës, por nuk lejon routing/swap. Objektivi 56 është kufi shard-esh në registry, jo kërkesë për t'i kaluar të gjitha në një transaksion.

Inicializimi përgatit vetëm shard-et 0..7 dhe vault-et/pozicionet e tyre me hapa të kufizuar nga madhësia/CU. Finalizimi kontrollon dëshmitarët për të tetë, pool/generation, statusin funded-ready dhe alokimet LP përpara se të publikojë atomikisht statusin e pool-it dhe `active_mask = 0xff`. Dështimi/riprova nuk lë një pool pjesërisht të finalizuar. Nëse llogaritë nuk futen në një transaksion, përdoret protokoll përgatitjeje me witness të versionuar që nuk mund të ndryshohet pa e zhvlerësuar; madhësia duhet të matet, jo të supozohet. Nuk parapaguhet rent për 56 shard-e.

Registry mbetet readonly në swap. Aktivizimi/çaktivizimi është operacion i rrallë writable që mund të konfliktojë me këta lexues. Ndryshimi i bit-it, statusit dhe dëshmisë së gatishmërisë duhet të jetë atomik ose i kontrolluar nga i njëjti version kanonik. `funded-ready` përmbledh kontrolle reale të vault-it, inventarit të nevojshëm për modelin dhe pretendimeve LP; një flag i vendosur nga administratori nuk i zëvendëson ato. Një lane aktiv mund të kufizohet/refuzojë swap sipas inventarit/çmimit aktual; aktivizimi nuk garanton thellësi tregu për çdo madhësi.

TVL-ja e vetme nuk mjafton për readiness: pozicionet jashtë intervalit mund të rrisin TVL pa thellësi aktive. Politika duhet të përcaktojë tregti standarde/slippage për secilin drejtim, headroom brenda intervalit dhe inventar real për output-in. Pragjet në njësi native janë specifike për çiftin; pragjet USD kërkojnë kontrollet oracle/decimals të SEC-06. Routing me hash shpërndan kandidatë, por përdoruesit mund ta provojnë hash-in shumë herë; nuk është mbrojtje ndaj MEV ose garanci best execution.

**Shtimi i një shard-i të ri të financuar** krijon gjendje dhe pretendime LP nga kapital i ri i autorizuar, pa prekur pretendimet ekzistuese. **Split i një shard-i ekzistues** transferon inventar dhe pretendime ekzistuese dhe duhet të plotësojë SEC-10 dhe ruajtjen e detyrimeve; ndryshimi i maskës nuk është implementim i split-it. Në të dy rastet, heqja e bit-it çaktivizon routing-un e ri, por nuk anulon pronësinë ose të drejtën e tërheqjes së fondeve që mbeten.

## Migrimi dinamik: kufij projektimi

PDA-ja nuk ndryshon adresë: krijohen llogari të reja dhe transferohen fonde/pretendime. Llogaritë e prekura duhet të jenë deklaruar përpara transaksionit; programi nuk mund të zbulojë një shard të ri dhe ta shtojë në mes të ekzekutimit. Hot path duhet të kontrollojë gjeneratën e lane-it, edhe nëse directory është readonly. Një update i directory-t mund të konfliktojë përkohësisht me lexuesit.

Split në të njëjtin çmim mund të ndajë inventarin dhe pretendimet me rregulla eksplicite. Pas split-it, performanca dhe çmimet lokale mund të divergojnë. Merge në çmime të ndryshme kërkon settlement/ribalancim të përcaktuar ose vazhdim si grup micro-pools. Nuk pretendohet ruajtje falas e një NAV-je globale të saktë apo semantikë identike e një CLMM-je të vetme.

## Burime primare dhe lidhja me testet

- [Solana: Sealevel dhe llogaritë e deklaruara](https://solana.com/news/sealevel---parallel-processing-thousands-of-smart-contracts): baza e SEC-05 dhe SEC-13.
- [Solana: transaction pipeline](https://solana.com/docs/core/transactions/transaction-pipeline): payer writable, konfigurimi v1 dhe rollback; SEC-05/13.
- [Solana: transferime tokenësh](https://solana.com/docs/tokens/basics/transfer-tokens): burimi/destinacioni dhe autorizimi; SEC-01/03/11.
- [Kamino: handler-i i likuidimit](https://github.com/Kamino-Finance/klend/blob/master/programs/klend/src/handlers/handler_liquidate_obligation_and_redeem_reserve_collateral.rs): shembull me market readonly, obligation dhe rezerva/vault-e writable; SEC-05/07.
- [Pyth: best practices](https://docs.pyth.network/pro/best-practices): verifikimi kriptografik nuk zëvendëson freshness dhe kontrollin e përzgjedhjes së update-ve; SEC-06.

Statusi i provave mbahet në [acceptance.md](acceptance.md). Ky dokument përshkruan kërkesa; asnjë masë nuk quhet e implementuar vetëm sepse është shkruar këtu.
