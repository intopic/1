# ADR-0006 — `rebalance_pair` i hapur dhe bit-et e pauzës

**Statusi:** PROPOSED (implementuar dhe testuar) · **Data:** 2026-10-29

## Konteksti

Çdo shard ka çmimin e vet. Kur router-i dërgon një urdhër të vogël në një shard të vetëm (ENG-09), ai shard largohet nga të tjerët. Çmimet e ndryshme e prishin kuotën e urdhrit të ardhshëm dhe e lënë fitimin e arbitrazhit mes shard-eve për atë që e mbyll i pari.

Përdoruesi vendosi (29 tetor 2026) që balancimi të jetë **i hapur për këdo**, jo funksion ekskluziv i protokollit. Një bot në Fly.io mund ta thërrasë, por pa asnjë privilegj. Ai kërkoi edhe bit-e të veçanta pauze për swap-et, depozitat dhe balancimin, me kusht që daljet e LP-ve të mos bllokohen kurrë (ADR-0002).

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

### Bit-et e pauzës

`MarketConfig.pause_flags: u8` merr 1 bajt nga `reserved`:

| Bit | Ndalon |
|---|---|
| 0 | swap-et (dhe rebalancimin, që përbëhet nga swap-e) |
| 1 | shtimin e likuiditetit |
| 2 | rebalancimin |

`set_pause_flags(flags)`: pauser-i mund vetëm të **shtojë** bit-e, kurse admini mund t'i vendosë si të dojë. Kjo është e njëjta asimetri si te `set_paused`: një pauser i komprometuar mund të ndalojë, por jo të rihapë. Bit-et 3..7 refuzohen. `MarketStatus::Paused` mbetet çelësi i plotë. Asnjë bit nuk bllokon `decrease_liquidity`, `collect_fees` ose `close_position`.

## Modeli ekonomik

- **Burimi i fitimit.** Diferenca e çmimit mes dy shard-eve, që e krijon rrjedha e tregtarëve. Për t'u kryer, rebalancimi duhet të fitojë më shumë se dy tarifat. Ndryshe nuk ndodh.
- **LP-të.** Principali i asnjë LP-je nuk preket jashtë mekanikës normale të swap-it. Çdo shard bën saktësisht atë që do të bënte me dy swap-e të zakonshëm, me të njëjtat invariante të aftësisë paguese, dhe LP-të fitojnë tarifat e të dy hapave.
- **Protokolli.** Merr vetëm pjesën e zakonshme të tarifave. Nuk ka MEV "ekskluziv" për protokollin. Kushdo që gjen diferencën e mbyll atë, dhe konkurrenca ia jep pjesën më të madhe të fitimit validatorit ose thirrësit më të shpejtë. Këtë nuk e kemi matur.
- **Pse më mirë se dy swap-e të lira.** Nuk kërkon kapital, prandaj rrit numrin e atyre që mund të balancojnë. Garanton që çmimet nuk kryqëzohen. Shkruan vetëm dy shard-e, kështu që të gjashtë të tjerët mbeten të lirë (A-10).

## Alternativat e refuzuara

- **Funksion ekskluziv për keeper-in:** refuzuar nga përdoruesi. Do të krijonte një rol të privilegjuar që zgjedh kur dhe sa fiton.
- **Rebalancim me tarifë zero:** u heq LP-ve tarifat pikërisht në rrjedhën që i ka kushtuar.
- **Transferim i drejtpërdrejtë likuiditeti mes shard-eve:** prek principalin dhe pozicionet e LP-ve. Refuzuar.

## Evidenca

- `engine::rebalance` (3 teste host): të njëjtat rezultate si dy swap-e të thjeshtë në të dy drejtimet (pool-et dhe tick arrays të barabarta), refuzimi pa diferencë çmimi, pa fitim, me sasi që kalon kufirin dhe me prag fitimi, 400 rebalancime të rastit që afrojnë çmimet pa i kryqëzuar.
- `market_config::pause_flags_roles_and_effects`: rolet dhe efekti i çdo biti.
- `eng12_rebalance` (4 teste onchain): thirrësi pa asnjë token merr saktësisht fitimin e parashikuar nga motori host, dy shard-et përfundojnë ku parashikohet dhe 6 të tjerët mbeten byte-për-byte të pandryshuar. I njëjti rezultat si dy swap-e të thjeshtë në një treg identik, për të 8 shard-et dhe të gjitha vault-et. Refuzimet (i njëjti shard, pa diferencë, pa fitim, llogari e gabuar, sasi shumë e madhe, pauzë). Pauzat dhe rolet: pauser-i nuk mund të rihapë, daljet e LP-ve punojnë nën çdo bit.
- CU: rreth 69 000 për një rebalancim pa kalim tick-u.

Ende pa bërë: simulim i fitimit të rebalancimit me router-in me prag (hapi 4), dhe teste stresi me rebalancime të rastit onchain.
