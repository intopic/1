# Modeli i llogarive dhe lock-eve

> **Dokument historik (para implementimit).** Ruhet si kontekst. Kur ndryshon nga kodi ose nga dokumentet normative, vlejnë [numeric-spec.md](numeric-spec.md), [account-layout.md](account-layout.md), [tx-budget.md](tx-budget.md), [ADR-të](adr/README.md) dhe [STATUS.md](../STATUS.md). Indeksi: [docs/README.md](README.md).

Status: projektim paraprak për një program Solana të ardhshëm. Nuk është IDL apo kontratë ekzistuese. `R` = vetëm lexim; `W` = writable; `—` = nuk përfshihet. Roli signer është i veçantë nga R/W. Të gjitha llogaritë e CPI deklarohen nga klienti përpara ekzekutimit.

## Identitetet PDA të propozuara

Seed-et janë tentative. `shard_id` kodohet si një bajt `u8`; numrat e tjerë kodohen me gjerësi fikse little-endian (`u16/u32/u64` sipas kolonës); public keys janë 32 bajt. Duhet një version skeme dhe renditje kanonike e mint-eve; emrat nuk janë siguri pa verifikim të pronësisë dhe lidhjeve të llogarive.

| Llogaria | Seed-et e propozuara | Pronari/autoriteti |
|---|---|---|
| `MarketConfig` | `["market", schema_u16, mint_a, mint_b, market_id_u64]` | Programi AMM |
| `ShardRegistry` | `["registry", market]` | Programi AMM; market binding + maskat + versioni |
| `ShardState` | `["shard", market, shard_id_u8, generation_u64]` | Programi AMM |
| `VaultAuthority` | `["vault_auth", shard_state]` | PDA signer; mund të jetë pa data |
| `VaultA/B` | `["vault", shard_state, mint]` | SPL Token; authority = `VaultAuthority` |
| `TickArray` | `["ticks", shard_state, start_tick_i32]` | Programi AMM |
| `LpPosition` | `["lp_pos", shard_state, owner, tick_lower_i32, tick_upper_i32]` (normative: [account-layout.md](account-layout.md)) | Programi AMM |
| `ReserveLane` | `["reserve", market, lane_id_u32, generation_u64]` | Programi i huadhënies ose modul i të njëjtit program; vendim i hapur |
| `ReserveAuthority` | `["reserve_auth", reserve_lane]` | PDA signer |
| `DebtVault/CollateralVault` | `["reserve_vault", reserve_lane, mint]` | SPL Token; authority = `ReserveAuthority` |
| `DebtPosition` | `["debt_pos", reserve_lane, owner, position_id_u64]` | Programi i huadhënies |

`DebtVault` dhe `CollateralVault` duhet të kenë identitet të dallueshëm nëse mint-et mund të jenë të njëjta; MVP kërkon mint borxhi dhe kolaterali të ndryshëm. Në implementim final mund të përdoren seed-e me role eksplicite për të shmangur këtë kufizim.

Nisja AMM përdor `shard_id=0..7`; kapaciteti është `0..55`, me `generation=0` për çdo shard të ri dhe pa ripërdorim të ID+generation. ID-të e reserve lanes të huasë përcaktohen veçmas; nuk trashëgojnë automatikisht maskën AMM. Swap-i kontrollon gjeneratën e pritur dhe `Active`; prepare/seed kërkojnë statusin joaktiv përkatës dhe tërheqjet LP kanë rrugë të përcaktuar edhe kur shard-i është joaktiv. Shard state lidh market, mint-et, vault-et dhe autoritetin. Position lidh owner, shard/lane dhe gjeneratën përmes adresës së tij. Klienti nuk autorizon një zëvendësim llogarie vetëm sepse paraqet të njëjtat data.

## Matrica R/W e veprimeve

Tabela përshkruan një shard/lane të vetëm. Për route shumëshardësh merret bashkimi i seteve; çdo account shfaqet vetëm një herë me privilegjin më të fortë të nevojshëm. Nëse payer dhe authority janë e njëjta adresë, adresa mbetet `W signer`.

| Llogaria/roli | Swap | Add liquidity | Remove liquidity | Liquidate pa swap | Split i ardhshëm |
|---|---|---|---|---|---|
| Payer, signer | W | W | W | W | W |
| User authority, signer | R | R | R | R (likuiduesi) | R (operator i autorizuar) |
| `MarketConfig` | R | R | R | R | R; W vetëm në ndryshim të vetë config |
| `ShardState` i zgjedhur | W | W | W | — | W prindi dhe fëmijët |
| `VaultAuthority` | R | R | R | — | R për secilin shard |
| `VaultA/B` lokal | W | W | W | — | W prindi dhe fëmijët |
| Tick arrays që preken | W nëse ndryshon fee/tick state | W | W | — | W burimi/destinacioni |
| Tick arrays vetëm për discovery/verifikim | R, nëse nevojiten | R, nëse nevojiten | R, nëse nevojiten | — | R, nëse nevojiten |
| `LpPosition` i përdoruesit | — | W | W | — | W vetëm për migrim të kufizuar; lazy claims i pavendosur |
| Token source/destination të përdoruesit | W | W | W | W të likuiduesit | — |
| Mint A/B | R | R | R | — | R |
| `ReserveLane` i detyrimit | — | — | — | W | —; migrimi i huasë është operacion më vete |
| `ReserveAuthority` | — | — | — | R | — |
| `DebtPosition` i likuiduar | — | — | — | W | — |
| Vault borxhi + kolaterali i lane | — | — | — | W | — |
| Mint borxhi/kolaterali | — | — | — | R | — |
| Oracle feeds të konfiguruara | — | — | — | R | — |
| Oracle observation lokal AMM | W vetëm nëse aktivizohet | — | — | — | W në migrim nëse ekziston |
| Token program i pin-uar | R | R | R | R | R |
| Programi i thirrur dhe sysvars të nevojshme | R | R | R | R | R |
| System program | — | R në krijim pozicioni | — | — | R në krijim fëmijësh |
| `ShardRegistry` | R | R | R | R në route AMM | W kur ndryshon anëtarësia |

Krijimi/rritja e llogarive kërkon payer dhe programet përkatëse. Mbyllja e një pozicioni ose kthimi i rent-it kërkon destinacionin e lamports `W`, edhe nëse nuk paraqitet si token destination në tabelë. `Remove liquidity` përfshin arkëtim tarifash nga të njëjtat vault-e; arkëtimi i veçantë do të ketë të njëjtin set minimal pa ndryshimin e tick-eve kur kontabiliteti e lejon. Nuk ka treasury ATA globale të shkruar gjatë swap-it; tarifat e protokollit regjistrohen lokal dhe mblidhen me operacion të veçantë.

Likuidimi me swap shton të gjitha llogaritë nga kolona `Swap` për secilin shard të route-it. Një receipt mint i përbashkët për burn/mint, një oracle i rifreskuar me shkrim në të njëjtin transaksion ose flashloan global duhet shtuar si `W`: këto janë konflikte reale, jo përjashtime nga matrica.

## Kontrata e route-it

Klienti dërgon një listë të kufizuar `(shard, generation, tick_arrays, input_bound, output_bound)`; programi kontrollon renditjen, identitetet dhe mungesën e aliases së papritur. Llogaritë e tepërta writable krijojnë konflikt edhe nëse branch-i që i përdor nuk ekzekutohet. Mos deklaroni të tetë ose të 56 shard-et për çdo swap si masë paraprake.

Për tetë swap-e të pavarura të njëkohshme synohet:

- `W_i = {payer_i, state_i, vault_a_i, vault_b_i, user_source_i, user_dest_i, tick_arrays_i[, observation_i]}`;
- `R_i` përfshin konfigurimin e përbashkët, programet, mint-et dhe autoritetet;
- payer-at dhe token accounts që shkruhen janë të ndryshëm për secilin `i`.

Nëse i njëjti bot përdor të njëjtin payer/source/destination në të tetë transaksionet, ky skenar nuk vërteton pavarësi të plotë. Benchmark-u duhet të raportojë veçmas përdorues të pavarur dhe një likuidues të përbashkët.

## Registry me maskë u64 dhe operacionet e kontrollit

Të dhënat e propozuara janë `{market, active_mask:u64, prepared_mask:u64, registry_version:u64, bump}` plus discriminator/version skeme. `INITIAL_MASK=0xff`, `VALID_MASK=(1_u64<<56)-1`; të dy maskat duhet të kenë zero në bit-et `56..63`, dhe `active_mask & !prepared_mask == 0`. Përgatitur përfshin aktiv; nuk është set i veçuar nga aktivët. Prepared tregon ekzistencë/inicializim të verifikuar, jo financim aktual: aktivizimi/finalizimi rivlerëson likuiditetin, bilancet, tick headroom dhe pretendimet LP. Statusi i shard-it dhe bit-i aktiv duhet të përputhen.

`ShardRegistry` është `R` për çdo swap/add dhe route likuidimi që përdor AMM; është `R` edhe gjatë heqjes normale të likuiditetit. Dalja/shlyerja nga një shard i ngrirë ose joaktiv kërkon instrukson të posaçëm që nuk bllokon pretendimet LP përgjithmonë vetëm sepse bit-i nuk është aktiv. Maskat nuk rezervojnë lock-e: runtime përdor R/W të llogarive të deklaruara.

| Veprimi i kontrollit | Registry | State | Vault-e | Llogari shtesë/kusht |
|---|---|---|---|---|
| Përgatit një shard joaktiv | W: vendos prepared bit, rrit version | W: krijo `Prepared` | W: krijim i llogarive | Payer W, admin/keeper signer R, config/mints/programet R; financimi dhe LP claims në seed të veçantë |
| Financo shard të përgatitur | R | W: kontabilitet lokal | W | Token sources/LP pretendim W; mbetet joaktiv |
| Finalizo nisjen me 8 | W: active `0xff`, version++ | W: 8 state me ID `0..7` | R: të 16 vault-et për verifikim të bilanceve | Config W për status tregu; payer W; admin signer dhe programet R; asnjë transferim i 56 lane-ve |
| Aktivizo një ID të financuar | W: një active bit, version++ | W: `Prepared -> Active` | R për bilancet | Config R, admin/keeper i lejuar signer R, payer W; checks për mint/authority/thellësi |
| Deaktivizo një ID | W: pastro active bit, version++ | W: `Active -> Frozen/Inactive` sipas protokollit | R për verifikime; W vetëm në transferim të deklaruar | Pretendimet/tarifat/huatë nuk fshihen; pa mbyllje përpara shlyerjes |
| Rebalance i kufizuar dypalësh | R | W: dy shard-et | W: vault-et e prekura | Config R, keeper signer R, payer W; llogari LP/claims W kur detyrimet ndryshojnë; jo registry W për çdo transferim |

Përgatitja/aktivizimi kontrollon versionin e pritur të registry-t për të refuzuar operacione kontrolli të vjetruara. Swap-i kontrollon bit-in aktual, market status dhe gjeneratën e shard-it; nuk kërkon të shkruajë versionin global. Nuk dërgohen të gjithë state/vault-et për kontroll anëtarësie: registry i vetëm plus shard-i konkret mjaftojnë për këtë kontroll, krahas llogarive reale të swap-it.

Në implementimin Rust/Anchor, ID-ja kontrollohet `id < 56` përpara shift-it; shift-i dhe rritja e versionit përdorin aritmetikë të kontrolluar. `popcount` nuk është prova e një ID-je të vlefshme nëse bit-et e rezervuara nuk janë kontrolluar. Finalizimi i nisjes ka 8 state dhe 16 vault-e plus registry/config/programet; kjo është listë e kufizuar llogarish, por kalimi i kufijve CU/madhësi duhet verifikuar mbi transaksionin real. Registry shton një adresë te buxheti i çdo swap-i; kapaciteti 56 nuk është premtim për 56 shard-e brenda një transaksioni.

Routing-u hash përdor rendin e bit-eve aktive: `rank=hash % popcount(active_mask)` dhe `select_set_bit(mask, rank)`; numërimi zero refuzohet. `&7` është shortcut vetëm për maskën `0xff`; `&55` nuk bën modulo 56 dhe nuk përdoret. Përzgjedhja dhe maska e lexuar nuk japin autoritet për të ngarkuar PDA të padeklaruara.

## Gjeneratat dhe migrimi

Gjenerata është pjesë e identitetit të llogarisë, jo thjesht fushë që klienti mund ta kalojë ndryshe. Në split të ardhshëm, prindi i vjetër bëhet joaktiv atomikisht me regjistrimin e kalimit. Swap/add/remove për gjeneratën e vjetër refuzohen sipas protokollit të migrimit; kërkesat e migrimit/shlyerjes lejohen vetëm nga instruksoni i posaçëm.

Nëse migrimi kërkon disa transaksione, ngrirja dhe rezervimi i fondeve ndodh përpara aktivizimit të destinacionit për përdorim të atyre fondeve. Çdo LP claim duhet të ketë identitet dhe status konsumimi për të ndaluar shlyerjen dy herë. Versionimi i kuotave të huasë trajtohet me të njëjtën disiplinë. Derisa ky protokoll të përcaktohet dhe testohet, split mbetet jashtë MVP-së.

## Kontrollet para ABI-së përfundimtare

Programi duhet të verifikojë owner/discriminator, seeds+bump, versionin, market/shard/lane bindings, mint dhe token authority, signer-in real dhe statusin e aktivizimit. Duhet të refuzojë source/destination/vault alias që thyejnë matematikën dhe të mos pranojë token program apo oracle program arbitrar nga klienti. Për SPL Token-2022 duhet analizë e plotë e extension-eve dhe hook-eve përpara mbështetjes.

Verifikimi i implementimit duhet të prodhojë listën reale të llogarive për çdo instrukson dhe madhësinë e transaksionit pas kompilimit. Matrica këtu nuk është matje performance. Shih [arkitekturën](architecture.md) për burimet dhe vendimet ekonomike.

