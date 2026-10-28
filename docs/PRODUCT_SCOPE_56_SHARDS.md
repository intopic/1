# Objektivi i produktit: smart contract me 8 shard-e fillestare, deri në 56

Përditësuar më 28 tetor 2026 sipas planit të përdoruesit. Ky është specifikim objektivi dhe rishikim fizibiliteti, jo implementim i përfunduar. Arkitekti Solana dhe specialisti AMM shkëmbyen gjetjet; koordinatori verifikoi dokumentet Jupiter/Solana dhe llogaritjet.

## Çfarë ndërtojmë

Produkti kryesor është programi onchain Solana **Sharded-Tick AMM & Liquidation Engine**. Një instancë tregu **nis me 8 shard-e aktive** dhe lejon zgjerim dinamik deri në 56 pas financimit dhe kontrolleve të likuiditetit. Shard-et kanë vault-e, çmim dhe kontabilitet lokal. Maska `u64` nis me `0xff`; përdoren vetëm bitet 0..55. Boti keeper është shërbim ndihmës që thërret programin, jo produkt i veçantë dhe jo kod që ekzekutohet vetë brenda kontratës.

Vendimi me katër shard-e është zëvendësuar. Nisja është me tetë, ID 0..7; 1/2/4 përdoren vetëm si baseline testimi. Nuk krijohen ose financohen të gjithë 56 në nisje. Overview dhe struktura Rust/Anchor janë te [OVERVIEW_RUST_ANCHOR_8_TO_56.md](OVERVIEW_RUST_ANCHOR_8_TO_56.md). Huadhënia/likuidimi dhe split/merge kërkojnë specifikimet dhe portat e tyre ekzistuese.

## 1. Numri i PDA-ve nuk është numri i llogarive në transaksion

Modeli i propozuar SPL përdor për çdo shard:

- Një llogari gjendjeje programore.
- Një token vault për tokenin A dhe një për tokenin B.
- Tick arrays dhe, sipas modelit, autoritet/observime të tjera.

Nisja me 8 kërkon **24 llogari state/vault** në treg, përpara tick arrays dhe llogarive të tjera. Në kapacitetin 56 do të ishin **168**. Këto janë llogaritë ekzistuese të tregut, jo detyrim për t'i futur të gjitha në çdo transaksion. Përdorimi i dy vault-eve globale zvogëlon numrin e adresave dhe rikthen konfliktin në çdo swap.

Shembull numërimi projektues, jo transaksion i matur: bazë 9 adresa për payer=trader, dy user token accounts, config, registry, dy mint-e, token program dhe AMM program. Për k shard-e të prekura:

| Supozimi | Numri i adresave |
|---|---:|
| Gjendje + dy vault-e për shard | 9 + 3k |
| Më sipër + një tick array për shard | 9 + 4k |
| Më sipër + autoritet PDA më vete për shard | 9 + 5k |

Kufijtë optimistë për 64 adresa janë përkatësisht 18, 13 dhe 11 shard-e; CPI, CU, numri i instruksioneve dhe bajtet mund t'i ulin më tej. Këto nuk janë kufij të matur të kontratës. Edhe 96 adresa nuk do të mjaftonin për 168 llogaritë minimale të këtij modeli.

V1 ka 4,096 bajt dhe 64 adresa sipas kufirit të kontrolluar në këtë rishikim; ato vlejnë për të gjithë transaksionin. [Solana upgrade](https://solana.com/upgrades/larger-transaction-sizes)

**Vendimi aktual:** 8 bit-e aktive në nisje dhe kapacitet deri në 56 në regjistër; swap në një shard ose në një route të kufizuar; keeper bën rebalance dypalësh ose me grupe të vogla. Zgjerimi aktivizon vetëm shard-e të financuara, me registry read-only gjatë swap-eve. Një cikël i plotë i keeper-it mund të ketë disa transaksione dhe nuk është atomik globalisht. Çdo hap duhet të jetë solvent dhe të respektojë pretendimet LP.

## 2. Kufizimi i keeper-it nuk rezervon arbitrazhin e swap-eve publike

`require_keys_eq!(keeper, ADMIN_KEEPER_PUBKEY)` mbron instruksionin e privilegjuar të ribalancimit. Ai nuk e kufizon automatikisht instruksionin publik `swap`.

Kundërshembull matematikor i ekipit, pa tarifa dhe pa tick crossing:

```text
Shard A: X=100, Y=100
Shard B: X=100, Y=121
Bot i jashtëm:
  paguan 5 Y te swap(A) -> merr 4.761904... X
  shet këtë X te swap(B) -> merr 5.5 Y
Rezultat bruto: +0.5 Y, pa thirrur rebalance.
```

Me tarifa/slippage/kosto rrjeti mundësia zvogëlohet ose zhduket; ACL mbi `rebalance` nuk e bën ekskluzive. Dy swap-e në shard-e të ndryshme mund të futen në një transaksion të vetëm të jashtëm.

Nëse protokolli zotëron 100% të inventarit LP, fitimi i keeper-it që vjen vetëm nga tregtitë midis dy shard-eve të tij është transferim brenda pasurisë së vet: gjendja e keeper-it rritet dhe inventari i pool-eve ulet përkatësisht. Nuk është burim i ri vlere për pronarin e kombinuar. Kur ka LP të jashtëm, nxjerrja e kësaj vlere duhet të përshkruhet dhe të llogaritet ndaj të drejtave të tyre.

Nuk premtojmë 100% fitim arbitrazhi ose fitim pa rrezik. Politika e keeper-it, auction/rebate dhe shpërndarja e përfitimit kërkojnë vendim ekonomik të veçantë.

## 3. Jupiter është integrim dhe konkurrencë kuotash

Jupiter kërkon implementim SDK, quote parity kundrejt ekzekutimit dhe dorëzim për rishikim; dokumentet rendisin auditin, code health dhe traction ndër kriteret. Një trait i implementuar ose program i deploy-uar nuk garanton pranim apo volum. Nuk kemi verifikuar pretendimin se mbi 80% e volumit të të gjithë Solana-s kalon prej tij. [Integrimi zyrtar](https://developers.jup.ag/docs/swap/routing/amm/integration)

Trait-i ka më shumë metoda të detyrueshme sesa tre funksione; emri për ndërtimin e swap-it është `get_swap_and_account_metas`. [API e crate-it](https://docs.rs/jupiter-amm-interface/latest/jupiter_amm_interface/trait.Amm.html)

Pool-et duhet të plotësojnë kriteret e likuiditetit për të mbetur të routueshme. [Market listing](https://developers.jup.ag/docs/swap/routing/amm/market-listing)

`quote` punon mbi gjendje të cache-uar; nuk është rezervim i shard-it ose garanci që adresa është ende pa lock kur transaksioni arrin. Route-i nuk supozohet se merr automatikisht gjithë volumin e pool-eve të tjera gjatë ngarkesës. Duhet konkurruar në dalje neto, likuiditet dhe besueshmëri.

## 4. Performanca: hipoteza që duhet provuar

Nuk ka garanci “kurrë AccountInUse”, rezervim të katër threads ose rritje universale 4x. Payer, vault-e, user token accounts, treasury ATA, flashloan dhe observime të përbashkëta mund të krijojnë konflikte. Një rebalance i madh që shkruan shumë shard-e i bllokon ato gjatë transaksionit.

Për **katër zgjedhje të pavarura, uniforme dhe me zëvendësim nga 56 shard-e**, modeli birthday jep:

```text
P(të paktën një çift prek të njëjtin shard)
 = 1 - (56*55*54*53)/(56^4)
 = 10.366936953...%
```

Ky është një model probabilistik i përzgjedhjes. Nuk është matje e AccountInUse, përqindje e transaksioneve të dështuara ose provë për throughput. Router-i real mund të dërgojë shumë tregti te shard-i me çmimin më të mirë, duke krijuar hotspot.

Slot-i aktual 200 ms konfirmohet nga faqja zyrtare; nuk është premtim për inclusion/finality të çdo swap-i. [Reduced slot times](https://solana.com/upgrades/reduced-slot-times)

## 5. Tarifat dhe kostot

Ndarja e propozuar 80% LP / 20% protokoll mund të implementohet si parametër i kontrolluar. Për 1,000,000 USD volum, në supozimin e një notional të përbashkët:

| Tarifa totale | Tarifat bruto | 80% LP | 20% protokoll |
|---|---:|---:|---:|
| 4 bps = 0.04% | $400 | $320 | $80 |
| 5 bps = 0.05% | $500 | $400 | $100 |
| 25 bps = 0.25% | $2,500 | $2,000 | $500 |
| 100 bps = 1% | $10,000 | $8,000 | $2,000 |

Këto janë të ardhura bruto sipas formulës, jo fitim ose volum i garantuar. LP PnL përfshin ndryshimin e vlerës së inventarit, arbitrimin/adverse selection dhe kosto të tjera. Nivelet janë propozim për testim, jo tarifa optimale të provuara.

Tarifa e rrjetit paguhet nga fee payer i transaksionit, që mund të jetë përdoruesi ose sponsor. CPI nuk e faturon automatikisht programin AMM si payer më vete. Keeper-i paguan transaksionet e veta. Baza e dokumentuar është 5,000 lamports për nënshkrim; priority fee shtohet. [Solana fees](https://solana.com/docs/core/fees)

Kontabilizojmë protocol fees lokalisht për shard dhe i mbledhim në treasury me instruksion të veçantë. Transferimi në të njëjtën treasury token account në çdo swap do të rikthente writable account të përbashkët.

Në 30 ditë, një thirrje çdo dy sekonda do të ishte 1,296,000 transaksione; çdo sekondë 2,592,000. **Edhe sipas supozimit të përdoruesit $0.001/tx**, kjo do të ishte $1,296–$2,592/muaj vetëm rrjet, jo $5–$20 gjithsej. Me një nënshkrim dhe vetëm bazën 5,000 lamports, janë 6.48–12.96 SOL/muaj për ato frekuenca, pa prioritet ose retries. Ky nuk është çmim i matur i keeper-it tonë; një strategji event-driven mund të thërrasë shumë më rrallë.

Fly.io ka makina të vogla me kosto të ulët, por compute, storage dhe egress janë përbërës më vete. RPC, transaksionet dhe monitorimi duhet të buxhetohen veçmas. [Fly pricing](https://fly.io/pricing/)

Nuk është matur kostoja 3 SOL për produktin tonë. Rent-i varet nga madhësia reale e bytecode, gjendjes, tick arrays, vault-eve dhe pozicioneve; matet pas fiksimit të layout-it. Kapitali i likuiditetit, auditimi dhe operimi janë zëra të veçantë. [Rent RPC](https://solana.com/docs/rpc/http/getminimumbalanceforrentexemption)

## Dorëzimet që i korrespondojnë këtij qëllimi

1. Program Rust/Anchor me config, shard registry, state/vault-e të pavarura dhe pozicione LP.
2. Swap dhe fee accounting të testuara ndaj modelit reference.
3. Keeper me rebalance të kufizuar dhe ruajtje të detyrimeve LP; pa pretendim monopoli mbi swap-et publike.
4. Jupiter SDK adapter dhe parity suite, pastaj aplikim për integrim kur plotësohen kushtet.
5. Launch me 8; benchmark baseline 1/2/4 dhe aktivizime pasuese 8→9, 16/32/56 me kontrolle financimi dhe kapital total të krahasueshëm.
6. Moduli i huadhënies/likuidimit dhe sharding dinamik pas portave përkatëse.

Programi AMM, keeper-i dhe Jupiter adapter ende nuk janë implementuar. Kodi ekzistues mbetet model matematikor dhe programe për provën e mjedisit. Ky rishikim përditëson qëllimin e produktit pa ndryshuar artificialisht statusin e zbatimit.
