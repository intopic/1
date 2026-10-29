# ADR-0006 — `rebalance_pair` pa kapital, vetëm për rebalancer-at tanë, dhe bit-et e pauzës

**Statusi:** PROPOSED (implementuar dhe testuar; i kufizuar te rebalancer-at më 29 tetor 2026) · **Data:** 2026-10-29

## Konteksti

Çdo shard ka çmimin e vet. Kur router-i dërgon një urdhër të vogël në një shard të vetëm (ENG-09), ai shard largohet nga të tjerët. Çmimet e ndryshme e prishin kuotën e urdhrit të ardhshëm dhe e lënë fitimin e arbitrazhit mes shard-eve për atë që e mbyll i pari.

Fillimisht përdoruesi vendosi (29 tetor 2026, paradite) që balancimi të ishte i hapur për këdo. Më vonë po atë ditë e ndryshoi vendimin: **vetëm rebalancer-at e protokollit** mund ta thërrasin, sepse ky funksion u ndërtua për protokollin. Shih "Kush mund ta thërrasë" më poshtë. Ai kërkoi edhe bit-e të veçanta pauze për swap-et, depozitat dhe balancimin, me kusht që daljet e LP-ve të mos bllokohen kurrë (ADR-0002).

Sot kushdo mund të balancojë me dy swap-e të zakonshëm në një transaksion. Por i duhet kapital në tokenin hyrës, dhe asgjë nuk e ndalon të kalojë përtej çmimit të shard-it tjetër.

## Vendimi

### `rebalance_pair(input_is_a, amount_in, min_profit)`

Dy shard-e aktivë të të njëjtit treg, `lo` (çmimi më i ulët i A) dhe `hi`. Programi i rendit vetë sipas `sqrt_price`; çmime të barabarta refuzohen.

| `input_is_a` | Hapi 1 | Hapi 2 | Fitimi |
|---|---|---|---|
| `false` | në `lo`: B → A, `amount_in` B | në `hi`: A → B, i gjithë A-ja e hapit 1 | në B |
| `true` | në `hi`: A → B, `amount_in` A | në `lo`: B → A, i gjithë B-ja e hapit 1 | në A |

- **Kufijtë e çmimit.** Hapi 1 ka kufi çmimin e shard-it tjetër para veprimit. Hapi 2 ka kufi çmimin e shard-it të parë pas hapit 1. Prandaj çmimet afrohen dhe nuk kryqëzohen kurrë. Nëse sasia është shumë e madhe, swap-i dështon me `PriceLimitReached`, si çdo swap me kufi.
- **Pa kapital.** Thirrësi nuk paguan asgjë. Transfertat bëhen mes vault-eve të të dy shard-eve, dhe thirrësi merr vetëm diferencën `output_2 − amount_in`, që duhet të jetë `≥ min_profit` dhe `> 0`. Përndryshe gabim `RebalanceUnprofitable`.
- **Kontabiliteti.** Çdo shard përfundon me të njëjtën gjendje dhe të njëjtat balanca vault-esh si pas dy swap-eve të zakonshëm me ato sasi dhe ato kufij. Testi i ekuivalencës e kontrollon këtë byte për byte.
- **Tarifat.** Secili hap paguan tarifën normale të shard-it të vet (bazë ose dinamike, ADR-0005). Pjesa e protokollit dhe fee growth i LP-ve llogariten si në swap. Nuk ka tarifë të reduktuar dhe nuk ka shpërblim nga protokolli.
- **Llogaritë.** Thirrësi (signer), `market` dhe `registry` vetëm për lexim. Për çdo shard: `shard`, dy vault-et dhe tick arrays (W), plus `vault_authority`. Një llogari tokeni e thirrësit për fitimin. Tick arrays jepen si remaining accounts: `arrays_lo` të parat, pastaj të `hi`-së.
- **Event:** `RebalanceExecuted { shard_lo, shard_hi, input_is_a, amount_in, amount_mid, amount_out, profit, fee_lo, fee_hi, sqrt_price_lo, sqrt_price_hi }`.

### Kush mund ta thërrasë

`MarketConfig.rebalancers: [Pubkey; 4]`. Një vend bosh është `Pubkey::default()` dhe nuk përputhet me askënd. Në krijimin e tregut lista është `[keeper, bosh, bosh, bosh]`. Çdo thirrës tjetër, përfshirë adminin, merr `NotRebalancer`.

- `update_rebalancers(keys)` (admin) propozon listën e re. Çelësat e dyfishtë refuzohen me `InvalidRebalancers`. Lista bosh e ndal rebalancimin plotësisht.
- `apply_rebalancers` (admin) e zbaton pas `FEE_TIMELOCK_SECONDS` (24 orë), si ndryshimet e tarifave. `cancel_rebalancers_change` e anulon.
- Eventet: `RebalancersChangeProposed { keys, eta }` dhe `RebalancersUpdated { old, new }`.

**Pse mjafton kjo kontroll.** `rebalance_pair` bën vetëm dy swap-e me tarifë të plotë dhe kufij çmimi, prandaj edhe një çelës rebalancer i komprometuar nuk mund të bëjë asgjë që nuk mund ta bëjë çdo tregtar me swap-e të zakonshëm. Rreziku i një çelësi të vjedhur është humbja e fitimit të balancimit, jo e fondeve të LP-ve.

**Çfarë nuk jep.** Kushdo mund ta mbyllë ende diferencën mes shard-eve me dy swap-e të zakonshëm dhe kapitalin e vet. Kufizimi i jep protokollit vetëm rrugën pa kapital, jo ekskluzivitet të plotë. Nëse bot-i ynë ndalet, diferencat mbeten derisa dikush tjetër t'i arbitrojë me kapital, ose derisa admini të shtojë një çelës tjetër pas 24 orësh.

### Bit-et e pauzës

`MarketConfig.pause_flags: u8` merr 1 bajt nga `reserved`:

| Bit | Ndalon |
|---|---|
| 0 | swap-et (dhe rebalancimin, që përbëhet nga swap-e) |
| 1 | shtimin e likuiditetit |
| 2 | rebalancimin |

`set_pause_flags(flags)`: pauser-i mund vetëm të **shtojë** bit-e, kurse admini mund t'i vendosë si të dojë. Kjo është e njëjta asimetri si te `set_paused`: një pauser i komprometuar mund të ndalojë, por jo të rihapë. Bit-et 3..7 refuzohen. `MarketStatus::Paused` mbetet çelësi i plotë. Asnjë bit nuk bllokon `decrease_liquidity`, `collect_fees` ose `close_position`.

**Pauzë për një shard (29 tetor 2026, ide nga Raydium).** `ShardState.pause_flags` ka të njëjtat bit-e dhe ndryshohet me `set_shard_pause_flags(flags)`, me të njëjtat role: pauser-i vetëm shton, admini vendos çdo kombinim. Biti ndalon vetëm atë shard:
- swap-i refuzohet me `ShardPaused`;
- depozita e re dhe `open_position` refuzohen;
- `rebalance_pair` refuzohet kur njëri nga dy shard-et ka `PAUSE_SWAPS` ose `PAUSE_REBALANCE`.

Shard-et e tjera vazhdojnë, dhe daljet e LP-ve nga shard-i i ndaluar mbeten të hapura. Biti ruhet te vetë shard-i, të cilin swap-i e shkruan gjithsesi, kështu që nuk shtohet asnjë llogari e përbashkët. Router-i nuk e kuoton një shard me swap-e të ndaluara, dhe nëse megjithatë merr `ShardPaused`, gabimin e klasifikon si `SkipShard`.

**Rikuperimi dhe rihapja e kontrolluar (29 tetor 2026).** Kur një shard ndalet vetëm me `PAUSE_SWAPS`, ai hyn në gjendje rikuperimi:
- Swap-et dhe depozitat e reja refuzohen, ndërsa daljet e LP-ve mbeten të hapura.
- Rebalancer-at e listuar mund ta afrojnë çmimin e tij me `rebalance_pair`, por vetëm kundër një shard-i që po tregton. Dy shard-e të ndalura nuk mund të rebalancohen me njëri-tjetrin.
- Rebalancimi bën dy swap-e të vërteta, me tick crossings, L dhe tarifat e secilit shard, dhe programi nuk lejon që çmimet të kalojnë njëri-tjetrin. Madhësia kufizohet nga vetë diferenca e çmimit. Fitimi, pra diferenca pas dy tarifave, i shkon rebalancer-it tonë dhe nuk ka kufi të veçantë.
- `PAUSE_SWAPS | PAUSE_REBALANCE` e ngrin shard-in plotësisht. Daljet e LP-ve mbeten edhe atëherë.

`PAUSE_SWAPS` nuk mund të hiqet me `set_shard_pause_flags` (`ReopenNeedsPriceCheck`). Atë e heq vetëm `reopen_shard(flags)`, vetëm admini, dhe vetëm nëse në të njëjtin transaksion kalojnë këto kontrolle:
- jepen të paktën `MIN_REOPEN_REFERENCES` (3) shard-e reference, vetëm për lexim, aktive, që po tregtojnë, të ndryshme nga njëra-tjetra dhe nga vetë shard-i;
- çmimi i shard-it është brenda `readiness.max_activation_deviation_bps` nga **mediana** e çmimeve të tyre. Përndryshe rihapja dështon dhe shard-i mbetet i ndalur.

Mediana dhe jo një shard i vetëm, sepse edhe një referencë mund të jetë e devijuar. Kjo e **kufizon** ekspozimin ndaj çmimit të vjetër, por nuk e eliminon: tregu mund të lëvizë menjëherë pas rihapjes, dhe arbitrazhi mund të shfaqet përsëri. Pauza globale mbetet për incidente që prekin disa shard-e, tokenin ose një varësi të përbashkët. "Daljet nuk bllokohen" do të thotë që politika jonë nuk i bllokon kurrë. Transferi mund të dështojë gjithsesi nga freeze i tokenit ose nga mungesa e fondeve.

Router-i (`find_rebalance`) e përfshin shard-in në rikuperim, por jo një çift ku të dy shard-et janë të ndalur.

## Modeli ekonomik

- **Burimi i fitimit.** Diferenca e çmimit mes dy shard-eve, që e krijon rrjedha e tregtarëve. Për t'u kryer, rebalancimi duhet të fitojë më shumë se dy tarifat. Ndryshe nuk ndodh.
- **LP-të.** Principali i asnjë LP-je nuk preket jashtë mekanikës normale të swap-it. Çdo shard bën saktësisht atë që do të bënte me dy swap-e të zakonshëm, me të njëjtat invariante të aftësisë paguese, dhe LP-të fitojnë tarifat e të dy hapave.
- **Protokolli.** Merr pjesën e zakonshme të tarifave dhe fitimin e çdo rebalancimi që bëjnë çelësat e tij. Nuk është MEV ekskluziv: arbitrazhi me swap-e të zakonshëm mbetet i hapur për të gjithë, dhe sa nga fitimi mbetet te protokolli nuk e kemi matur.
- **Pse më mirë se dy swap-e të lira.** Nuk kërkon kapital. Garanton që çmimet nuk kryqëzohen. Shkruan vetëm dy shard-e, kështu që të gjashtë të tjerët mbeten të lirë (A-10).

## Alternativat e refuzuara

- **I hapur për këdo:** varianti i parë, i zbatuar dhe i testuar. U zëvendësua me vendim të përdoruesit, që fitimi i rrugës pa kapital të mbetet te protokolli.
- **Rol + hapje automatike për këdo pas N slot-esh me diferencë:** më kompleks; nuk u zgjodh.
- **Rebalancim me tarifë zero:** u heq LP-ve tarifat pikërisht në rrjedhën që i ka kushtuar.
- **Transferim i drejtpërdrejtë likuiditeti mes shard-eve:** prek principalin dhe pozicionet e LP-ve. Refuzuar.

## Evidenca

- `engine::rebalance` (3 teste host): të njëjtat rezultate si dy swap-e të thjeshtë në të dy drejtimet (pool-et dhe tick arrays të barabarta), refuzimi pa diferencë çmimi, pa fitim, me sasi që kalon kufirin dhe me prag fitimi, 400 rebalancime të rastit që afrojnë çmimet pa i kryqëzuar.
- `market_config::pause_flags_roles_and_effects`: rolet dhe efekti i çdo biti.
- `market_config::rebalancer_list_behind_the_timelock`: vetëm admini, dyfishimet, timelock-u, anulimi, lista bosh.
- `eng12_rebalance` (4 teste onchain): vetëm keeper-i në krijim; një thirrës i panjohur dhe admini refuzohen me `NotRebalancer`; pas listimit funksionon; pas heqjes nga lista refuzohet përsëri. Rebalancer-i pa asnjë token merr saktësisht fitimin e parashikuar nga motori host, dy shard-et përfundojnë ku parashikohet dhe 6 të tjerët mbeten byte-për-byte të pandryshuar. I njëjti rezultat si dy swap-e të thjeshtë në një treg identik, për të 8 shard-et dhe të gjitha vault-et. Refuzimet (i njëjti shard, pa diferencë, pa fitim, llogari e gabuar, sasi shumë e madhe, pauzë). Pauzat dhe rolet: pauser-i nuk mund të rihapë, daljet e LP-ve punojnë nën çdo bit. `shard_pause_stops_one_shard_only` kontrollon:
  - rolet dhe bit-et e panjohura;
  - swap-i dhe depozita në shard-in 1 refuzohen me `ShardPaused`;
  - rebalancimi përmes shard-it 1 refuzohet, përmes shard-it 2 punon;
  - shard-et 3–7 mbeten byte-për-byte të paprekur dhe vazhdojnë të tregtojnë;
  - LP-ja del nga shard-i 1;
  - pauser-i nuk mund ta heqë pauzën, dhe as admini me `set_shard_pause_flags`, sepse duhet `reopen_shard`;
  - pas `reopen_shard` me referencat 3–5, shard-i punon përsëri.

  `controlled_reopen_after_recovery` kontrollon:
  - shard-i 0 është mbi 1% larg kur ndalet;
  - `reopen_shard` refuzohet për devijim, për më pak se 3 referenca, për vetë shard-in ose një dublikatë si referencë, për referencë të ndalur, dhe kur e thërret dikush tjetër përveç adminit;
  - rebalancimi kundër një shard-i tjetër të ndalur refuzohet;
  - rebalancer-i e afron çmimin kundër shard-eve të hapura, pa kapital, dhe merr vetëm fitimin;
  - më pas `reopen_shard` kalon dhe swap-et punojnë.
- CU: rreth 69 000 për një rebalancim pa kalim tick-u.

Ende pa bërë: simulim i fitimit të rebalancimit me router-in me prag (hapi 4), dhe teste stresi me rebalancime të rastit onchain.
