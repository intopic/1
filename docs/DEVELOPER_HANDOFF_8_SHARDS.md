# Developer handoff — Sharded-Tick AMM, 8 shard-e fillestare dhe kapacitet 56

**Versioni:** 1.0 · **Data:** 28 tetor 2026 · **Fusha:** specifikim për implementim Rust/Anchor.

Ky dokument i dorëzohet developer-it si përkufizim pune, arkitekture dhe provash. Nuk është kod i përfunduar, auditim ose certifikim sigurie. Nisja me **8 shard-e aktive** është kërkesë e produktit; kapaciteti **56** është kufiri i zgjerimit. Asnjë deklaratë për 8× throughput, fitim automatik ose origjinalitet ekskluziv nuk konsiderohet e provuar.

## 1. Rezultati që duhet të dorëzojë developer-i

Ndërto një program Solana që organizon një çift tokenësh si **tetë CLMM lokale të pavarura nën një konfigurim dhe regjistër të përbashkët**. Çdo shard ka çmimin, likuiditetin aktiv, tick arrays, vault-et, tarifat dhe pozicionet LP të veta. Më vonë mund të aktivizohen shard-e të tjera të financuara, deri në 56, pa zëvendësuar programin për çdo shtim.

Konfigurimi dhe regjistri lexohen gjatë swap-it; gjendja e shkruar kufizohet te shard-i i zgjedhur dhe llogaritë e përdoruesit. Router-i dhe keeper-i janë shërbime ndihmëse. Programi onchain kontrollon çdo autorizim, kufi çmimi, llogari, detyrim dhe ndryshim kapitali.

**Dorëzimi fillestar:** build i riprodhueshëm, IDL, SDK, tetë shard-e të financuara në validator/devnet, swap/LP/fees funksionale, suite adversariale dhe raport matjesh. Huadhënia/likuidimi dhe migrimi i kapitalit ekzistues janë faza të veçanta; emri i produktit nuk i bën të implementuara.

## 2. Çfarë ekziston sot në folder

| Pjesa | Statusi real |
|---|---|
| `crates/amm-model` | Model kërkimor integer constant-product; 5 teste të ekzekutuara me sukses. Nuk është CLMM me ticks/pozicione/vault-e |
| `env-smoke` | Program ping/pong pa fonde; build SBF i ekzekutuar me sukses |
| `env-anchor` | Projekt i vogël për verifikim Anchor/IDL; i përgatitur, ende i pandërtuar |
| `scripts/` | Kontrollet lokale dhe setup/prova Linux/Windows |
| `docs/architecture.md`, `account-model.md`, `security.md`, `acceptance.md` | Specifikim dhe matrica të projektit, me statuset e provave |
| Programi AMM me tetë shard-e | Ende i paimplementuar |

Pengesa lokale e regjistruar është virtualizimi BIOS i fikur për WSL2; shih `STATUS.md`. Developer-i mund të përdorë një mjedis Linux funksional për build/test. Mos e përdor mungesën e mjedisit si arsye për të deklaruar kontratën të testuar.

## 3. Raydium dhe Orca: çfarë kemi verifikuar

### 3.1 Referenca kryesore e lejueshme për shqyrtim ripërdorimi

Raydium CLMM ka repository publik dhe licencë Apache 2.0 në snapshot-in e verifikuar:

`ed7c84a54ced59c55981780546adb0b4583dcf85`

Ky është snapshot i riprodhueshëm, **jo deklaratë që përfaqëson HEAD-in ose deployment-in më të fundit**. [Commit-i](https://github.com/raydium-io/raydium-clmm/commit/ed7c84a54ced59c55981780546adb0b4583dcf85), [LICENSE në të njëjtin commit](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/LICENSE).

Për kod të ripërdorur, ruaj licencën, atribuimet dhe NOTICE përkatës kur ekziston, dhe shëno ndryshimet. Regjistro skedarin dhe commit-in e origjinës. Kjo nuk jep emrin/markën e Raydium ose garanci sigurie për versionin tonë.

### 3.2 Orca nuk trajtohet si fork komercial i lirë i versionit aktual

Whirlpools ka kod publik, por LICENSE aktual është **Orca License**, me kushte për përdorimin komercial dhe konkurrentët; nuk është i barasvlershëm me Apache 2.0 pa kufizime. Për rrugë ripërdorimi kërkohet pëlqimi i zbatueshëm ose verifikim i veçantë i një commit-i historik, licencës dhe prejardhjes së çdo skedari. Nuk kemi importuar implementim Orca. [Repository](https://github.com/orca-so/whirlpools), [LICENSE aktual](https://github.com/orca-so/whirlpools/blob/main/LICENSE).

Mos përdor etiketën “open source” si autorizim automatik për kopjim komercial. Rruga bazë e këtij projekti është implementim i dokumentuar mbi matematikë të specifikuar dhe referenca Raydium të verifikuara, me rishikim licence për çdo portim.

### 3.3 Auditimet janë referenca, jo trashëgimi sigurie

Ekzistojnë listime raportesh në [Raydium/OtterSec Q3 2022](https://github.com/raydium-io/raydium-docs/tree/master/audit/OtterSec%20Q3%202022), [Raydium/Sec3 Q2 2026](https://github.com/raydium-io/raydium-docs/tree/master/audit/Sec3%20Q2%202026) dhe [Orca `.audits`](https://github.com/orca-so/whirlpools/tree/main/.audits). U verifikua ekzistenca e artefakteve; raportet nuk u analizuan këtu për gjetje, scope ose përputhje me deployment-in aktual.

Developer-i duhet të lidhë çdo raport me commit-in e mbuluar, ndryshimet dhe korrigjimet përkatëse. Arkitektura jonë me maskë dhe shumë shard-e kërkon testim dhe auditim të vetin.

## 4. Harta konkrete e shembujve Raydium

Të gjitha lidhjet më poshtë janë të fiksuara në snapshot-in e mësipërm. Ato janë materiale reference, jo listë skedarësh të kopjuar në projekt.

| Skedari upstream | Çfarë duhet studiuar | Si përshtatet te ne |
|---|---|---|
| [create_pool.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/instructions/create_pool.rs) | Inicializimi, renditja e mint-eve, identiteti PDA/vault/token program | Krijim për çdo shard; regjistri dhe hapja me tetë janë logjika jonë |
| [pool.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/states/pool.rs) | Modelimi lokal i çmimit, L dhe tarifave | Një `ShardState` për shard, jo një gjendje writable për gjithë tregun |
| [swap_v2.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/instructions/swap_v2.rs) | Lidhjet mes config, mint, authority, vault dhe tick accounts | Shto market/shard/generation/bit membership; mos trashëgo kompleksitetin Token-2022 pa scope |
| [token.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/util/token.rs) | CPI të kontrolluara, decimals dhe PDA signer seeds | Fikso programin token dhe autoritetin e vault-it lokal |
| [tick_array.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/states/tick_array.rs) | Indekset, kufijtë dhe lidhja e vargut me pool-in | Çdo varg lidhet me shard-in/gjeneratën. Byte order i seed-eve është vendim i ABI-së sonë |
| [personal_position.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/states/personal_position.rs) | Range, L, fee checkpoints dhe owed balances | Pozicioni ynë lidhet direkt me owner PDA; modeli NFT nuk kopjohet automatikisht |
| [increase_liquidity.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/instructions/increase_liquidity.rs) / [decrease_liquidity.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/instructions/decrease_liquidity.rs) | Validimi position/pool/tick/vault dhe llogaritja LP | Ndryshimi kufizohet te një shard; pretendimet historike ruhen |
| [swap_math.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/libraries/swap_math.rs) | Swap-step, rounding, price target dhe teste të përfshira | Referencë për diferenciale kundrejt modelit tonë të pavarur |
| [full_math.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/libraries/full_math.rs) / [tick_math.rs](https://github.com/raydium-io/raydium-clmm/blob/ed7c84a54ced59c55981780546adb0b4583dcf85/programs/amm/src/libraries/tick_math.rs) | Saktësia ndërmjetëse dhe lidhja tick/çmim | Provo domain-in dhe konstantet; mos përziej Q64.64 me Q96 |

Nuk trashëgohen automatikisht packed layouts, `unsafe`, llogari kompatibiliteti të vjetruara, rewards, NFT positions ose të gjitha token extensions. Çdo element duhet të ketë arsye, rishikim dhe test.

## 5. Vendimet e arkitekturës që nuk ndryshohen pa rishikim

1. Një program menaxhon tregun; launch me tetë shard-e të financuara, ID 0–7.
2. Kapaciteti është 56, ID 0–55; bitet 56–63 janë të rezervuara dhe zero.
3. Çdo shard ka dy token vault-e të veçanta. Nuk ka vault global writable në çdo swap.
4. Çmimet dhe L janë lokale; nuk imponojmë një çmim global të menjëhershëm.
5. `MarketConfig` dhe `ShardRegistry` janë read-only gjatë swap/LP normal.
6. Protocol fees akumulohen lokalisht; treasury collection është instruksion i veçantë.
7. Pozicionet LP lidhen me shard-in; kapitali nuk numërohet në dy shard-e njëkohësisht.
8. Aktivizimi me kapital të ri nuk është migrim i LP-ve ekzistuese.
9. Rebalance është i kufizuar; nuk planifikohet `rebalance_all_56` atomik.
10. Router-i nuk është autoritet financiar. Programi verifikon kufijtë e nënshkruar dhe gjendjen reale.

Për tetë swap-e që synojnë paralelizëm, `W_i` përfshin payer/user token accounts lokale, `ShardState_i`, vault-et dhe tick arrays përkatëse. Config/registry/mint/program mund të jenë lexime të përbashkëta. Duhet të provohet `W_i ∩ (R_j ∪ W_j)=∅` për çdo çift të pavarur. I njëjti payer ose user ATA mund ta shkelë këtë edhe me shard-e të ndryshme.

## 6. Skema e llogarive dhe ABI-ja

| Llogaria | Fushat e kërkuara konceptualisht |
|---|---|
| `MarketConfig` | schema version, mint A/B kanonike, token program, admin/keeper/revenue authority, fee params, growth policy, market status, bump |
| `ShardRegistry` | market, `prepared_mask:u64`, `active_mask:u64`, `registry_version:u64`, bump |
| `ShardState` | market, `id:u8`, generation, lifecycle status, vault pubkeys, sqrt price X64, current tick, active L, fee growth A/B, protocol fees A/B, dust policy, bump |
| `TickArray` | shard/generation binding, start tick, spacing/domain, tick entries me gross/net L dhe fee-growth-outside |
| `LpPosition` | owner, shard/generation, position ID, lower/upper ticks, position L, fee-growth-inside checkpoints, owed fees |

Për L/çmim/fee-growth parashikohen `u128`, për token amounts `u64`, për ticks `i32`; developer-i duhet të fiksojë domain-in, madhësinë e layout-it dhe politikat e overflow përpara kodit. Enum-et, padding, discriminator, account sizes dhe çdo ndryshim skeme dokumentohen. Asnjë `Vec` ose input pa kufi në gjendje/instruksione për të shmangur rritje të pakontrolluar të madhësisë.

PDA seeds normative për AMM-në e re:

```text
market:     ["market", schema_u16_le, mint_a_bytes, mint_b_bytes, market_id_u64_le]
registry:   ["registry", market_pubkey]
shard:      ["shard", market_pubkey, id_u8, generation_u64_le]
vault_auth: ["vault_auth", shard_pubkey]
vault:      ["vault", shard_pubkey, mint_pubkey]
tick_array: ["ticks", shard_pubkey, start_tick_i32_le]
position:   ["lp_pos", shard_pubkey, owner_pubkey, tick_lower_i32_le, tick_upper_i32_le]
```

> **Përditësim (auditimi F-02):** varianti i mëparshëm me `position_id_u64_le` u zëvendësua. ABI normative është në [account-layout.md](account-layout.md), me fixture adresash në `docs/vectors/pda_fixtures.json`.

Ky është ABI i programit tonë; nuk supozohet kompatibilitet adresash me Raydium. Në veçanti, seed byte order i tick arrays nuk merret pa shqyrtim nga upstream-i. Inicializimi i vault-eve përdor token program-in e lejuar dhe autoritetin lokal; vault authority nuk ka të drejtë administratori për të transferuar principal arbitrarisht.

## 7. Bitmask-i dhe cikli i jetës

```rust
pub const INITIAL_SHARDS: u8 = 8;
pub const MAX_SHARDS: u8 = 56;
pub const INITIAL_MASK: u64 = (1u64 << INITIAL_SHARDS) - 1; // 0xff
pub const VALID_MASK: u64 = (1u64 << MAX_SHARDS) - 1;
```

Kontrollo `id < 56` përpara shift-it. Refuzo bitet e rezervuara; përdor `count_ones()` për numrin aktiv dhe kërko `active_mask & !prepared_mask == 0`. `prepared_mask` provon përgatitje/identitet, jo likuiditet aktual. `registry_version` rritet me kontroll overflow në operacionet e kontrollit; nuk shkruhet për çdo swap.

Nisja: `Initializing → Open`. Përgatit dhe financo secilin nga tetë shard-et në hapa të kufizuar; `finalize_initial_shards` rivlerëson të tetë dhe vendos atomikisht statuset aktive dhe maskën `0xff`. Nuk pranohet swap në një treg të hapur pjesërisht. Setup i ndërprerë ka retry idempotent dhe rrugë kthimi të fondeve të depozituesit.

Zgjerimi: përgatit një ID të papërdorur `<56`, financoje me kapital real/pozicione LP, pastaj aktivizoje vetëm pasi kalon kontrollet aktuale. Një LP që tërheq fonde pas prepare-it mund të ndryshojë readiness; aktivizimi nuk i beson një flag-u historik. Shtimi i një ID-je të re nuk ndryshon pretendimet e LP-ve të vjetra.

Versioni i parë kërkon zgjerim me kapital të ri; `deactivate_shard`, ripërdorimi i ID-ve dhe tkurrja automatike janë jashtë këtij dorëzimi. Tërheqjet nga shard-e të përgatitura por ende joaktive dhe nga gjendja pause duhet të specifikohen/testohen që në fillim. Nëse shtohet më vonë deactivation, ndryshimi status/maskë është atomik dhe nuk anulon withdraw/collect ose pretendimet e mbetura.

Router-i mund të zgjedhë ID sipas kuotës ose rank-ut të një biti aktiv. `hash & 55` ndalohet; `hash & 7` vlen vetëm për maskën prefiks `0xff`. `hash % count` kërkon pastaj select të bitit aktiv sipas rank-ut dhe refuzim të maskës zero. Hash routing nuk është best execution ose kufi sigurie.

## 8. Kontrata e instruksioneve

| Instruksioni | Kush e autorizon | Kontrolli/dorëzimi kryesor |
|---|---|---|
| `initialize_market` | admin | Mint ordering, params/domain të vlefshëm, registry bosh, status joaktiv |
| `prepare_shard` | admin/keeper me rol të kufizuar | ID/generation/PDA unike, vault-e të vlefshme, prepared bit dhe version |
| `seed_liquidity` | financuesi LP | Transferime reale, range/L dhe pretendim LP; shard joaktiv mbetet joaktiv |
| `finalize_initial_shards` | roli i hapjes | Tetë ID 0–7 me likuiditet/inventar/claims të vlefshme; status + maskë atomike |
| `swap_exact_in` | authority e token input | Amount, min-out, price limit, deadline, id/generation dhe të gjitha lidhjet; registry R |
| `add_liquidity` | owner/depositor sipas pozicionit | Amount max A/B, ticks/range, historic fees settled, token deltas dhe L |
| `remove_liquidity` | owner i pozicionit | Amount min A/B, L i mjaftueshëm, historic fees, output te destinacionet e autorizuara |
| `collect_lp_fees` | owner | Owed claims konsumohen vetëm një herë; pa principal të fshehur si fee |
| `activate_shard` | admin/keeper sipas politikës | Kapacitet, përgatitje, likuiditet aktual, claims, price/readiness; bit + status atomik |
| `collect_protocol_fees` | revenue authority | Vetëm detyrimi i protokollit, në marrësit e fiksuar për mint-et përkatëse |
| `rebalance_pair` | keeper nëse instruksion i posaçëm | Dy shard-e, generation/deadline/min-out; pa transferim arbitrar LP; registry R |
| `pause` / param update | role të ndara | Bounds, event, version dhe politika e daljes; jo drain i vault-eve |

Për çdo handler dorëzo: args të versionuara, `Context<...>`, listën R/W/signer, event-et, error codes, kufijtë e CU/accounts dhe invarianta para/pas. Mos përdor `UncheckedAccount` pa invariantë të shkruar dhe teste negative. Refuzo alias-e të palejuara dhe renditje tick accounts që mund të ndryshojnë llogaritjen.

**Politikë fillestare e propozuar për exact-in:** i gjithë input-i konsumohet ose transaksioni refuzohet nëse price limit, inventari ose tick accounts nuk lejojnë përfundimin. Partial fills mund të shtohen vetëm me kontratë API/SDK eksplicite për input-in e konsumuar; mos i prezanto në heshtje.

## 9. Specifikimi matematikor që duhet mbyllur

Për shard-in i nuk kërkohet `price_i == price_j`. Likuiditeti aktiv duhet të barazohet me shumën e L të pozicioneve aktive në tick-un lokal. Token balances reale, principal claims, LP fees, protocol fees dhe dust duhet të pajtohen sipas një modeli të shkruar; rezervat virtuale nuk përdoren si provë e aftësisë së vault-it për pagesë.

Përdor një specifikim të vetëm Q64.64 për sqrt price, domain tick dhe fee growth, të fiksuar me test vectors. Nuk kopjohen konstante Q96. Për çdo formulë dokumento input bounds, bit-width të produkteve, drejtimin floor/ceil, zero divisors dhe cast-et e kontrolluara. Vetëm shprehja “përdorim U256” nuk mjafton: produkte naive me L `u128`, sqrt price `u128` dhe faktor `2^64` mund të kërkojnë deri në 320 bit. Përdor factorization/full-precision të rishikuar ose ndërmjetës më të gjerë dhe referencë të pavarur BigInt/racionale.

Swap loop: gjej tick-un tjetër të inicializuar në llogaritë e deklaruara; clamp target-in te kufiri i përdoruesit; llogarit hapin në dy drejtimet; akumulo tarifat me L përpara crossing-ut; përditëso fee-growth-outside/tick-net dhe L sipas drejtimit. Çdo iteracion konsumon input, lëviz çmimin, kalon tick ose përfundon. Mungesa e arrays/gap/limit nuk lejon loop të pafund. Përcakto kufirin tick në barazi dhe sjelljen për L=0.

Fee accounting:

> **E zëvendësuar (auditimi F-03).** Formula Q64 më poshtë ishte propozimi fillestar dhe **nuk** është ajo që implementohet. Normative është [numeric-spec.md](numeric-spec.md) seksioni 6: fee growth në **Q128** si U256 (`[u64; 4]`, limb-i 0 më pak i rëndësishëm), `inside` modulo 2^256, dhe `owed_delta = floor(old_L · Δinside / 2^128)` me ndërmjetës 512-bit. Vektorët `fee_growth`, `fee_inside` dhe `u256_limbs` në `docs/vectors/clmm_vectors.json` e fiksojnë.

```text
# HISTORIKE, jo normative:
LP_fee = total_fee - protocol_fee
growth_delta(token) = floor(LP_fee(token) * 2^64 / active_L)
owed_delta(position) = floor(old_position_L * inside_growth_delta / 2^64)
```

Llogarit tarifat historike me **L e vjetër** përpara ndryshimit të pozicionit. Tarifa nuk shton automatikisht active L. Remainder/dust ka pronar dhe rregull të deklaruar. Për fee-growth accumulator zgjidh dhe testo shprehimisht politikën modulare ose refuzimin në overflow; asnjë wrapping aksidental për amount/L/balance. Transfero tokenët vetëm pasi rezultatet përfundimtare dhe kufijtë janë validuar; dështimi CPI duhet të rrëzojë ndryshimet.

Fillo me SPL Token klasik dhe pa token extensions të pambuluara. Token-2022, hooks, transfer fees, rewards dhe NFT receipts janë zgjerime me analiza/teste të veçanta.

## 10. Rritja sipas likuiditetit, jo vetëm sipas TVL-së

Keeper-i vëzhgon thellësinë e tregtueshme në të dy drejtimet, slippage për madhësi tregtie të përcaktuara, ngarkesën, inventarin dhe kostot. Kontrata verifikon minimumet që mund të përkufizohen në mënyrë deterministe. TVL jashtë range-it aktiv nuk zëvendëson depth.

Përpara `activate_shard` duhet të fiksohen: njësitë e pragjeve, range/tick headroom, inventari real, politika e çmimit fillestar për shard-in e ri, burimi i financimit, tolerancat dhe cooldown. Pragjet në USD kërkojnë oracle të përcaktuar me decimals/freshness/confidence; për testin fillestar mund të përdoren pragje në njësi mint për çiftin e zgjedhur. Mos krijo një oracle të sigurisë së huasë nga çmimi i manipulueshëm i vetë shard-it.

Ndezja e një biti nuk krijon kapital. Kalimi nga tetë në më shumë pa kapital të ri mund të ulë depth për shard. Split i kapitalit ekzistues kërkon migrim të çdo pozicioni/range/tick/fee claim dhe gjenerata njëpërdorimëshe; është milestone i veçantë. Tërheqja/collect e vlefshme nuk duhet të humbasë vetëm sepse shard-i është joaktiv.

## 11. Keeper, Jupiter dhe kufijtë ekonomikë

Keeper-i propozon transaksione të kufizuara sipas përfitimit neto dhe riskut; nuk dërgon verbërisht rebalance çdo slot. Thirrjet kushtojnë tarifa, mund të dështojnë dhe nuk kanë ekzekutim të garantuar. Kufizimi i `rebalance_pair` te keeper-i nuk ndalon arbitrimin përmes dy swap-eve publike. Marrësit/shpërblimet janë të fiksuara ose të kufizuara nga politika; keeper-i nuk merr principal LP si “fitim” arbitrar.

Adapter-i Jupiter implementon trait-in real të versionit të fiksuar, përdor vetëm gjendje të dorëzuar/cache-uar dhe nuk bën network calls brenda quote/update implementation. Jep quote identike me onchain, account metas minimale dhe numërim të saktë të llogarive. Dorëzo parity suite për dy drejtimet, shumica madhësish tregtie, fees, tick crossing dhe gjendje stale. Integrimi kërkon rishikim dhe nuk garanton volum. [Jupiter integration](https://developers.jup.ag/docs/swap/routing/amm/integration)

Kontrata të jetë CPI-compatible dhe pa varësi të panevojshme nga formati i transaksionit. Klienti v1 ndërton konfigurimin compute/data dhe e mat mesazhin; versioni v1 nuk krijon paralelizëm brenda një transaksioni. Të 56 shard-et nuk kalohen në çdo swap. Kufijtë maten në cluster/runtime të fiksuar, përfshirë CPI-të dhe instruksionet totale.

## 12. Struktura e kodit që duhet krijuar

```text
programs/sharded-amm/src/
  lib.rs
  constants.rs, errors.rs, events.rs
  state/
    market_config.rs, shard_registry.rs, shard_state.rs
    tick_array.rs, lp_position.rs
  math/
    bitmap.rs, fixed_point.rs, full_math.rs
    tick_math.rs, swap_math.rs, liquidity_math.rs, fee_math.rs
  validation/
    account_links.rs, tick_accounts.rs, shard_readiness.rs
  instructions/
    initialize_market.rs, prepare_shard.rs, seed_liquidity.rs
    finalize_initial_shards.rs, activate_shard.rs
    swap_exact_in.rs, add_liquidity.rs, remove_liquidity.rs
    collect_lp_fees.rs, collect_protocol_fees.rs, rebalance_pair.rs
    pause.rs, update_parameters.rs
crates/
  amm-model/              # ekziston; zgjerohet me reference të pavarur
  amm-sdk/                # quote/encoding/account metas
  jupiter-adapter/        # trait + parity suite
keeper/                   # shërbim offchain, pa privilegj drain
tests/                    # unit, SVM, integration, adversarial, load
docs/
  numeric-spec.md, account-layout.md, instruction-spec.md
  operations.md, deployment.md, incident-response.md
  upstream-lock.json, third-party-notices.md
```

Pema është dorëzim i kërkuar, jo përshkrim që këta module tashmë ekzistojnë. Handler-at orkestrojnë validim, matematikë dhe CPI; matematika nuk varet nga RPC/Clock/llogari mutable. Komentet dokumentojnë invarianta, njësitë dhe drejtimin e rounding. Gabimet janë të emërtuara; `unwrap`, `panic`, `unsafe` dhe cast-et ngushtuese në rrugën e programit kërkojnë eliminim ose arsyetim të posaçëm dhe teste.

## 13. Plani i punës për developer-in

| ID | Detyra | Definition of done |
|---|---|---|
| ENG-00 | Mjedisi/reproducibility | Linux funksional, versione të fiksuara, build SBF dhe Anchor/IDL të përsëritshme; log dhe hashes |
| ENG-01 | ADR/numeric spec/provenance | Zgjedhjet e seksionit 16 të shkruara; snapshot-et/licencat dhe bit-width proofs të rishikuara |
| ENG-02 | Bitmap dhe lifecycle | Unit/property tests për maska, bounds, rank, zero, 55/56/63/255 dhe aktivizim të dyfishtë |
| ENG-03 | Accounts/PDA/token custody | Initialization + tetë shard-e të izoluara; teste për çdo owner/mint/authority/seed/alias të gabuar |
| ENG-04 | Launch tetësh | Prepare/seed staged, retry dhe finalizim atomik `0xff`; pa fonde ose LP claims të humbura |
| ENG-05 | Math/ticks/LP | Referencë e pavarur, exact boundaries, fee checkpoints, add/remove, konservim dhe fuzzing |
| ENG-06 | Swap dhe fees | Exact-in në dy drejtimet, min-out/limit/deadline, CPI rollback, protocol collection lokale |
| ENG-07 | SDK dhe parity | Quote = token deltas të programit; serialized bytes dhe account metas të numëruara |
| ENG-08 | Zgjerimi | Aktivizo ID 8 dhe më tej; prova 16/32/56 pas financimit, race/rollback/stale readiness |
| ENG-09 | Keeper/benchmark | Politikë bounded, min-out, kosto; fixtures uniform/hotspot dhe raport me kapital të barabartë |
| ENG-10 | Release engineering | IDL/SDK/source hashes, provenance/SBOM, udhëzim deploy/monitoring/incidenti dhe gjetje sigurie të mbyllura |

Çdo milestone dorëzon kod, prova, kufizime dhe komandat e riprodhimit. Një PR nuk mbyllet vetëm sepse build-i kalon. Ndryshimet në matematikë, custody dhe privileged instructions marrin rishikim të pavarur nga autori. Në fillim nuk ka arsye të shtohen UI, token protokolli ose shërbime të tjera jashtë këtij scope-i.

## 14. Testet që duhet të ekzistojnë

**Matematikë:** differential test kundrejt BigInt/racional, të dy drejtimet, ticks ekzaktë, max/min çmim, likuiditet zero/gap, rounding dust, input shumë i vogël dhe produkte pranë kufijve. Kërko që çdo seed dështimi të ruhet dhe të minimizohet.

**Maskë/lifecycle:** launch me vetëm tetë të përgatitura nuk lejohet pa financim; finalizim i pjesshëm refuzohet; bitet e rezervuara refuzohen; ID e njëjtë nuk aktivizohet dy herë; version overflow refuzohet; tërheqja para activation e detyron rishikimin e readiness; ndërprerja/riprova nuk dyfishon claims.

**Custody/LP:** user/vault alias, mint/token program/authority i gabuar, llogari e shard-it tjetër, tick arrays me rend/lidhje të gabuar, generation stale, kolektim i dyfishtë fee, donacione direkte në vault dhe përpjekje për t'i kthyer në LP shares falas.

**SVM/integration:** token balances para/pas, rollback pas CPI failure, tërheqje gjatë pause/inactive sipas politikës, ndryshime admin brenda bounds, kontrolle të roleve. Krahaso byte-for-byte shard-et e paprekura; instruksioni lokal nuk duhet t'u ndryshojë gjendjen.

**Paralelizëm:** validator real, users/payers/ATA të ndryshme, trafik uniform dhe trafik që zgjedh shard-in me kuotën më të mirë. Kontroll negativ me payer/vault të përbashkët dhe me të gjitha tregtitë në një shard. Simulimi SVM serial ose ndërthurja e modelit nuk provon scheduler throughput.

Launch mbetet me tetë; baseline 1/2/4 krijohet vetëm për krahasim. Për 8/16/32/56 përdor kapital total, range distribution dhe fees të krahasueshme. Regjistro runtime/hardware/CPU, seed ngarkese, concurrency, warmup, CU/bajtë, success/retry, p50/p95/p99 landing latency, dalje neto dhe rezultat LP. Ndaji dështimet e programit nga drop/expiry/AccountInUse. Përcakto pragjet e suksesit përpara matjes; raporti përfshin edhe rastet ku sharding del më keq.

## 15. Siguria operative dhe dorëzimi për GitHub/release

Ndaji upgrade authority, admin parametrash, keeper dhe revenue authority. Për kapital real përcakto multisig, vonesën e upgrade-ve dhe procedurën e njoftimit. Fikso fee recipients, bounds dhe pausing policy; asnjë handler ose rol operacional në kodin e specifikuar nuk merr të drejtë arbitrare mbi principal LP. **Upgrade authority mund të zëvendësojë kodin dhe të ndryshojë garancitë e custody-t**: multisig/vonesa e kontrollojnë këtë pushtet, por nuk e heqin. Ky besim duhet deklaruar dhe rishikuar për release-in.

Upgrade-i teston deserializimin e gjendjes së vjetër dhe migrimin. Rollback i bytecode nuk supozohet se zhbën migrimin e data layout-it. Publiko source/IDL/build hash për revision-in e release-it. Auditimi i pavarur mbulon kodin dhe ekonominë e ndryshuar; mos përdor auditin e upstream-it si certifikatë tonën.

GitHub duhet të përmbajë source, lockfiles, test fixtures të lejuara, dokumente, CI dhe notices. `.tools`, target binaries/logs, wallet/keypairs, sekretet dhe credentials nuk publikohen. Përpara push bëhet kontrolli i përmbajtjes/staged files; repository nuk është krijuar ose publikuar nga kjo detyrë.

`upstream-lock.json` duhet të ruajë për çdo skedar të importuar: repo, commit, path, hash, licencë, modifikime lokale dhe audit-scope kur njihet. Në këtë fazë nuk është importuar kod reference dhe ky manifest është dorëzim i ardhshëm, jo evidencë e sajuar.

Monitoro inventory/accounting divergence, failures/expiry, registry/status mismatch, depth për shard, protocol-fee claims, LP claims dhe përdorimin e roleve. Keeper-i ndalet kur sheh gjendje të paqartë dhe nuk vazhdon të paguajë retries pa kufi. Deploy-i fillon në validator/devnet; pilot me kapital real kërkon porta të veçanta dhe buxhet të matur.

## 16. Vendimet që developer-i duhet t'i fiksojë përpara handlers financiare

- Çifti fillestar, decimals, token program dhe politika për freeze/delegate/extensions.
- Domain tick/sqrt price/L, spacing, rounding, full precision dhe fee-growth wrapping policy.
- Fee denominator, pjesa e protokollit dhe rregulli për dust; p.sh. 80/20 është parametër i propozuar, jo ligj universal.
- Çmimi/range fillestar i tetë shard-eve dhe politika e çmimit për aktivizime të reja.
- Madhësia e trade-it standard, depth/minimum seed/cooldown dhe oracle nëse përdoren pragje USD.
- Bounds për tick arrays/route, sjellja kur exact-in nuk mund të konsumohet dhe përputhja SDK.
- Role matrix, pauza selektive, dalja e LP-ve nga shard joaktiv dhe upgrade/migration policy.
- Mekanizmi ekonomik i rebalance që nuk cenon LP principal.

Zgjedhjet regjistrohen si ADR me alternativat, arsyen, rreziqet dhe testet përkatëse. Mungesa e informacionit nuk plotësohet me supozime të fshehta në kod.

## 17. Kriteri për të thënë “MVP e përfunduar”

Tetë shard-e aktive me maskë `0xff`; state/vault/LP/ticks të izoluara; swap dhe operacione LP të sakta; protocol fees lokale; teste adversariale dhe diferenciale; build/IDL/SDK të riprodhueshme; prova validator/devnet; raport performance/ekonomike me kufizime; dokumente role/deploy/incident. Zgjerimi i financuar deri në 56 ka prova të veçanta, jo vetëm konstante në kod.

Emërtimi “unik”, “i pari”, “zero contention” ose “8×” nuk është kriter pranimi dhe nuk është verifikuar. Elementi që duhet të ruajmë në implementim është **grafi i pavarur i tetë shard-eve dhe zgjerimi i kontrolluar**, ndërsa cilësia matet me provat e mësipërme.

Për skicat Rust të maskës, strukturën e plotë të propozuar dhe detajet e lifecycle-it, shih [overview 8→56](OVERVIEW_RUST_ANCHOR_8_TO_56.md). Për gjendjen reale të progresit shih [STATUS](../STATUS.md); për portat e provave shih [acceptance](acceptance.md). Ky handoff nuk ndryshon automatikisht asnjë test nga `NOT IMPLEMENTED` në `PASS`.
