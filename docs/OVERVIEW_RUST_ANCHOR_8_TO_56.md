# Overview: kontratë AMM Rust/Anchor — nisje me 8 shard-e, zgjerim deri në 56

**Vendim i përdoruesit, 28 tetor 2026:** tregu hapet me **8 shard-e aktive**, ID `0..=7`. Kapaciteti maksimal është **56**, ID `0..=55`. Zgjerimi bëhet kur ka likuiditet të përdorshëm dhe kalojnë kontrollet e kontratës. Ky dokument zëvendëson propozimin e pilotit me katër shard-e.

**Status:** overview dhe plan implementimi. Struktura dhe fragmentet Rust më poshtë janë të propozuara; programi AMM, maska dhe zgjerimi ende nuk janë implementuar. Kodi aktual i testuar është modeli `crates/amm-model`; `env-smoke` dhe `env-anchor` janë projekte për provën e mjedisit.

## 1. Produkti dhe përgjegjësitë

| Pjesa | Çfarë bën |
|---|---|
| Smart contract Rust/Anchor | Mban konfigurimin, regjistrin, shard-et, vault-et, pozicionet LP, swap-et, tarifat dhe kontrollet e zgjerimit |
| Keeper | Vëzhgon likuiditetin/ngarkesën dhe propozon prepare/activate ose rebalance; çdo transaksion kalon kontrollet onchain |
| SDK/router | Lexon maskën dhe kuotat; ndërton transaksion vetëm me llogaritë e shard-eve të zgjedhura |
| Jupiter adapter | Përkthen gjendjen/kuotat dhe account metas në ndërfaqen e Jupiter-it; kërkon parity tests dhe rishikim integrimi |
| Moduli i likuidimit | Shtohet pasi kalon AMM-ja; përdor obligations dhe reserve lanes të izoluara, me oracle/risk accounting të veçantë |

Kontrata nuk zgjohet vetë kur rritet likuiditeti. Keeper-i ose një përdorues i autorizuar dërgon instruksionin; kontrata vendos nëse ai është i vlefshëm. Ruhet objektivi i tarifave swap dhe mbledhjes së protocol fees. Kufizimi i keeper-it nuk premton monopol mbi arbitrazhin nga swap-et publike.

## 2. Maska `u64`

Përdorim një bit për ID shard-i. Bitet `0..55` janë të vlefshme; `56..63` mbeten zero. Maska është indeks i anëtarësisë/aktivizimit, jo mekanizëm lock-u ose rezervim CPU.

```rust
pub const INITIAL_SHARDS: u8 = 8;
pub const MAX_SHARDS: u8 = 56;
pub const INITIAL_MASK: u64 = (1u64 << INITIAL_SHARDS) - 1;
pub const VALID_MASK: u64 = (1u64 << MAX_SHARDS) - 1;
// INITIAL_MASK = 0x0000_0000_0000_00ff
// VALID_MASK   = 0x00ff_ffff_ffff_ffff

pub fn shard_bit(id: u8) -> Option<u64> {
    if id >= MAX_SHARDS { return None; }
    Some(1u64 << id)
}

pub fn is_active(mask: u64, id: u8) -> bool {
    shard_bit(id).is_some_and(|bit| mask & bit != 0)
}
```

Operacionet kryesore:

```text
aktivizo id:   active_mask |= 1u64 << id
çaktivizo id:  active_mask &= !(1u64 << id)
numri aktiv:   active_mask.count_ones()
vlefshmëria:  active_mask & !VALID_MASK == 0
```

Këto operacione lejohen vetëm pas kontrolleve të account/status/fondeve. Shprehjet nuk janë autorizim për të aktivizuar një shard bosh.

Shembuj të mundshëm të fazave, jo kërkesë që çdo zgjerim të bëhet atomikisht në grup:

| Shard-e aktive në prefiks | `active_mask` |
|---:|---|
| 8 | `0x0000_0000_0000_00ff` |
| 16 | `0x0000_0000_0000_ffff` |
| 32 | `0x0000_0000_ffff_ffff` |
| 56 | `0x00ff_ffff_ffff_ffff` |

Kontrata mbështet edhe maska me boshllëqe. Fillimisht ID-të janë `0..7`; zgjerimi zgjedh një ID ende të papërdorur. Mbyllja, çaktivizimi dhe ripërdorimi i një ID-je nuk përfshihen automatikisht në versionin e parë.

**Mos përdor `hash & (active_count - 1)` në përgjithësi.** `hash & 7` punon vetëm për prefiksin fillestar prej 8 bitesh; `hash & 55` nuk shpërndan saktë në 56 shard-e dhe nuk zgjidh boshllëqet.

Për shpërndarje të thjeshtë jashtë zinxhirit mund të zgjidhet `rank = hash % active_count`, pastaj biti aktiv i asaj radhe:

```rust
pub fn active_id_at_rank(mask: u64, rank: u32) -> Option<u8> {
    if mask & !VALID_MASK != 0 || rank >= mask.count_ones() {
        return None;
    }
    let mut remaining = mask;
    for _ in 0..rank {
        remaining &= remaining - 1; // hiq bitin më të ulët të ndezur
    }
    Some(remaining.trailing_zeros() as u8)
}
```

Router-i ekonomik duhet të krahasojë kuotat: hash routing nuk garanton çmimin më të mirë. Kontrata merr `shard_id` eksplicit dhe verifikon bitin, PDA-në, statusin dhe gjeneratën. Nuk i beson zgjedhjes së klientit si provë sigurie.

## 3. Llogaritë onchain

| Llogaria | Fusha kryesore | A shkruhet në swap? |
|---|---|---|
| `MarketConfig` | admin, keeper, mint A/B, token program, fee params, politika e zgjerimit, statusi | Jo |
| `ShardRegistry` | market, `prepared_mask`, `active_mask`, `registry_version`, bump | Jo |
| `ShardState` | market, id, generation, status, vault A/B, sqrt price, active L, tick, fee growth | Po, vetëm shard-i i zgjedhur |
| `VaultA/B` | token balances reale dhe autoriteti lokal | Po, vetëm vault-et e zgjedhura |
| `TickArray` | liquidity gross/net dhe fee state për ticks | Vetëm vargjet që preken |
| `LpPosition` | owner, shard, range, L, fee checkpoints/claims | Vetëm operacionet LP |

Skicë e regjistrit Anchor:

```rust
#[account]
#[derive(InitSpace)]
pub struct ShardRegistry {
    pub market: Pubkey,
    pub prepared_mask: u64,
    pub active_mask: u64,
    pub registry_version: u64,
    pub bump: u8,
}
```

`prepared_mask` tregon se llogaritë janë përgatitur/valide; nuk është provë e mjaftueshme që likuiditeti aktual plotëson pragun. Aktivizimi e kontrollon përsëri gjendjen dhe inventarin real.

Invarianta të regjistrit:

```text
active_mask & !VALID_MASK == 0
prepared_mask & !VALID_MASK == 0
active_mask & !prepared_mask == 0
active_mask.count_ones() <= 56
hapja e tregut: active_mask == INITIAL_MASK
```

Nuk ruajmë një `active_count` të dytë që mund të dalë jashtë sinkronizimit: llogaritet me `count_ones()`.

PDA-të e propozuara:

```text
market:      ["market", schema, mint_a, mint_b, market_id]
registry:    ["registry", market]
shard:       ["shard", market, shard_id_u8, generation_u64_le]
vault_auth:  ["vault_auth", shard]
vault:       ["vault", shard, mint]
ticks:       ["ticks", shard, start_tick_i32_le]
position:    ["lp_pos", shard, owner, tick_lower_i32_le, tick_upper_i32_le]
```

Seed-et normative janë në [account-layout.md](account-layout.md) (auditimi F-02).

ID është `u8`; ndërrimi i gjerësisë së seed-it më vonë ndryshon adresën dhe kërkon versionim. Mint-et kanë renditje kanonike. Çdo instruksion kontrollon owner, seeds/bump, mint, authority dhe lidhjet market/shard. Fushat e çmimit/likuiditetit dhe aritmetika fixed-point përcaktohen plotësisht para implementimit CLMM; emërtimi i tyre këtu nuk përbën matematikë të përfunduar.

## 4. Nisja me tetë shard-e

Procesi ndahet në transaksione të kufizuara:

1. `initialize_market`: krijon config/registry me status `Initializing`, maska zero dhe parametrat e aprovuar.
2. `prepare_shard(id)`: për secilin ID `0..7`, krijon/verifikon state, autoritet dhe vault-e; vendos prepared bit. Statusi mbetet joaktiv.
3. `seed_liquidity(id)`: depoziton kapital real dhe krijon pretendimet LP të financuesit për atë shard. Tick/range dhe fee accounting duhet të jenë të vlefshme.
4. `finalize_initial_shards`: verifikon tetë shard-et, fondet, minimumet dhe gjendjen aktuale; vendos statuset aktive dhe `active_mask = 0xff`, pastaj tregun `Open` në të njëjtin transaksion finalizimi.

Vetëm pas hapjes lejohen swap-et. Të gjitha llogaritë që përdor finalizimi deklarohen paraprakisht. Duhet matur buxheti real i tij; nëse kontrollet nuk futen në CU/accounts, përshtatet protokolli i provës së readiness pa lejuar hapje me shard-e bosh. Nuk pretendojmë që krijimi dhe financimi i të tetëve duhet të futet në një transaksion.

Nëse setup-i ndërpritet, mund të vazhdojë në mënyrë idempotente. Depozituesi ka rrugë të përcaktuar për rikuperim/tërheqje përpara hapjes; statusi joaktiv nuk i heq të drejtat LP.

## 5. Swap dhe tarifa

`swap_exact_in` merr sasinë, min-out, kufirin e çmimit, afatin, ID-në dhe gjeneratën. Lexon config/registry; ndryshon vetëm state/ticks/vault-et lokale dhe token accounts të përdoruesit. Maskën **nuk e ndryshon gjatë swap-it**. Aktivizimi i një shard-i tjetër nuk duhet ta bëjë automatikisht të pavlefshme kuotën e shard-it ekzistues për shkak të një versioni global; çmimi/min-out/statusi kontrollohen në ekzekutim.

Parametrat e tarifës janë të kontrolluar, p.sh. 5 bps total dhe `protocol_share_bps = 2_000` nga 10,000 për ndarjen 20% protokoll / 80% LP. Këto janë parametra fillestarë për testim, jo nivele optimale të provuara. Tarifat akumulohen për shard. `collect_protocol_fees` bën transferimin në treasury më rrallë; nuk shkruajmë një treasury token account globale në çdo swap.

## 6. Zgjerimi 8 → deri në 56

Keeper-i vëzhgon thellësinë aktive, madhësitë e tregtimeve, slippage, shpërndarjen e ngarkesës dhe koston e mbajtjes së shard-eve. Politikë e propozuar: prag aktivizimi i shoqëruar me periudhë stabiliteti/cooldown dhe minimum kapitali për shard; vlerat nuk janë vendosur ende.

TVL e lartë nuk mjafton: likuiditeti jashtë intervalit aktiv nuk financon domosdoshmërisht swap-in aktual. Matja mund të përdorë një trade standard në secilin drejtim dhe slippage maksimal. Pragjet në USD kërkojnë oracle/decimals/freshness; për MVP mund të përdoren njësi tokenësh dhe intervale të përcaktuara për çiftin, pa pretenduar një vlerë USD universale.

Për një shard të ri:

1. Zgjidh një ID të lirë `<56`; kontrollo kapacitetin dhe autorizimin.
2. Përgatit PDA/vault/ticks në mënyrë joaktive.
3. Financuesi shton likuiditet dhe merr pozicione lokale LP.
4. `activate_shard` verifikon përsëri statusin, gjeneratën, inventarin, minimumet dhe pretendimet. Ndryshon atomikisht bitin dhe statusin; rrit `registry_version` me kontroll overflow dhe lëshon event.
5. SDK/keeper/Jupiter adapter rifreskojnë maskën dhe kuotat nga eventi/gjendja.

Zgjerimi nuk krijon kapital. Të drejtat LP të shard-eve ekzistuese nuk transferohen te financuesi i ri. **Transferimi i fondeve të LP-ve ekzistuese në shard-in e ri është split/migrim**, me procedurë të veçantë për pozicionet, tarifat dhe generation claims; nuk kryhet vetëm duke ndezur një bit.

Zgjerimi me kapital të ri është mekanizmi i parë dinamik. Split/merge dhe çaktivizimi me detyrime të pambyllura janë faza më të vonshme. Një pauzë sigurie, nëse shtohet, nuk duhet të zhdukë rrugën e shlyerjes/tërheqjes.

## 7. Instruksionet e propozuara

| Instruksioni | Qëllimi | Regjistri |
|---|---|---|
| `initialize_market` | Config dhe registry fillestar | W |
| `prepare_shard` | Krijim i shard-it joaktiv | W |
| `seed_liquidity` | Financim dhe pozicion LP për shard joaktiv | R; nuk e shpall automatikisht aktiv |
| `finalize_initial_shards` | Hapja atomike me tetë shard-e aktive | W |
| `swap_exact_in` | Swap lokal me min-out | R |
| `add_liquidity` / `remove_liquidity` | Menaxhim LP brenda shard-it | R |
| `collect_lp_fees` | Tarifa e pozicionit LP | R |
| `activate_shard` | Zgjerim vetëm pas verifikimit të financimit | W |
| `rebalance_pair` | Operacion i kufizuar mes dy shard-eve, me ruajtje të detyrimeve | R |
| `collect_protocol_fees` | Mbledhje lokale në treasury | R |
| `liquidate_position` | Fazë huadhënieje me risk/oracle/rezerva të veçanta | R në regjistrin AMM kur ka swap |

Ribalance nuk duhet të jetë thjesht transferim arbitrar i vault-eve të LP-ve. Mekanizmi i parë mund të përdorë swap-e normale të keeper-it mbi dy shard-e, me buxhet/min-out; çdo privilegj për transferim të drejtpërdrejtë kërkon specifikim ekonomik shtesë. `rebalance_all_56` nuk është instruksion i planit.

## 8. Struktura e propozuar Rust/Anchor

Pema më poshtë është struktura që do të krijojmë për kontratën; folderët `programs/sharded-amm`, SDK dhe keeper nuk ekzistojnë ende si implementim. Projekti `env-anchor` mbetet provë e veçuar e mjedisit.

```text
sharded_amm/
  Anchor.toml
  Cargo.toml
  programs/sharded-amm/
    Cargo.toml
    src/
      lib.rs                         # dispatcher instruksionesh Anchor
      constants.rs                   # INITIAL_SHARDS=8, MAX_SHARDS=56
      errors.rs                      # kapacitet, maskë, status, llogari, matematikë
      events.rs                      # MarketOpened, ShardPrepared, ShardActivated
      state/
        mod.rs
        market_config.rs
        shard_registry.rs
        shard_state.rs
        lp_position.rs
        tick_array.rs
      math/
        mod.rs
        bitmap.rs                    # bit/id/count/rank; pa token transfer
        fixed_point.rs
        swap_math.rs
        liquidity_math.rs
        fee_math.rs
        growth_policy.rs             # validim i minimumit, jo scheduler/bot
      instructions/
        mod.rs
        initialize_market.rs
        prepare_shard.rs
        seed_liquidity.rs
        finalize_initial_shards.rs
        swap_exact_in.rs
        add_liquidity.rs
        remove_liquidity.rs
        collect_lp_fees.rs
        activate_shard.rs
        rebalance_pair.rs
        collect_protocol_fees.rs
      validation/
        mod.rs
        account_links.rs
        shard_readiness.rs
  crates/
    amm-model/                       # ekziston: kerneli reference kërkimor
    amm-sdk/                         # e ardhshme: kuota + transaksione
    jupiter-adapter/                 # e ardhshme: trait dhe parity suite
  keeper/                            # e ardhshme: trigger-e zgjerimi/rebalance
  tests/
    bitmap.rs
    launch_eight_shards.rs
    swap_isolation.rs
    activation.rs
    fees_and_lp_claims.rs
    account_substitution.rs
    benchmark_scenarios.rs
  env-smoke/                         # ekziston: provë SBF pa AMM
  env-anchor/                        # ekziston: provë Anchor ende e pandërtuar
  docs/
  scripts/
```

`math` dhe `validation` ndahen nga handlers që të mund të krahasohen me modelin reference. SDK nuk duhet të ketë një formulë tjetër fee/rounding nga programi. Feature-t dhe versionet Anchor/Solana mbeten të fiksuara sipas `toolchain.versions.json`; Cargo/Anchor workspace final ndryshohet vetëm kur kontrata krijohet realisht.

## 9. Plan implementimi dhe prova

| Faza | Rezultati i kërkuar |
|---|---|
| A — Specifikim/bitmap | Invarianta, tetë bitet fillestare, cap 56, rank-selection dhe raste të pavlefshme; implementim/teste Rust reale |
| B — Hapja e tregut | Prepare/seed për 0..7, finalizim, vault-e të veçanta, rikuperim i setup-it të ndërprerë |
| C — AMM me tetë shard-e | Swap/LP/fees/ticks, konservim inventari dhe differential tests; gjendje të tjera të pandryshuara |
| D — Matjet | Launch me 8; kontrolle 1/2/4 për baseline; trafik uniform/hotspot dhe routes të kufizuara, me kapital total të barabartë |
| E — Zgjerimi | Aktivizime 8→9 dhe më tej; skenarë 16/32/56, pragje/cooldown, dështim/retry pa bit ose detyrim të dyfishtë |
| F — Integrimi | Keeper, Jupiter quote parity dhe rishikim integrimi; matje kostosh reale |
| G — Likuidimi/migrimi | Rezerva, oracle, kuota, borxh i keq; pastaj split/merge me LP claims të ruajtura |

Testet kritike: ID 7 dhe 8, ID 55, refuzimi 56/63/255; maska zero, bitet e rezervuara, maska me boshllëqe; aktivizim pa fonde; tërheqje pas përgatitjes dhe para aktivizimit; dy aktivizime për të njëjtin ID; version overflow; pause/status/generation; aktivizim gjatë swap-it; asnjë shkrim registry në swap; minimum depth në dy drejtime dhe ruajtje e detyrimeve LP.

Kjo tabelë është plan testimi. Nuk janë krijuar ose kaluar ende këto teste onchain; evidenca reale vazhdon të mbahet te `STATUS.md` dhe `docs/acceptance.md`.

## 10. Kufijtë që mbeten pjesë e dizajnit

Tetë shard-e nënkuptojnë të paktën 24 llogari state/vault në treg; 56 nënkuptojnë të paktën 168. Një swap i zakonshëm deklaron vetëm shard-in e zgjedhur. Kufijtë e transaksionit dhe CU maten për çdo instruksion; nuk premtojmë se të gjithë shard-et ribalancohen atomikisht. Bitmask nuk zgjeron kufijtë runtime dhe nuk garanton mungesë absolute të `AccountInUse`.

Pragjet ekonomike, aritmetika CLMM e plotë dhe migrimi LP kanë nevojë për specifikim të përfunduar. Çdo shifër fee/performancë/kosto mbetet konfigurim ose hipotezë deri në matje. Burimet e verifikuara për Solana/Jupiter dhe kufijtë ekonomikë janë te [specifikimi i produktit](PRODUCT_SCOPE_56_SHARDS.md); matrica e llogarive te [account-model.md](account-model.md).
