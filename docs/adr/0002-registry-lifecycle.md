# ADR-0002 — Registry me bitmask dhe lifecycle

**Statusi:** PROPOSED; logjika është implementuar dhe testuar në host (ENG-02) · **Data:** 2026-10-28

## Vendimi

- `ShardRegistry { market, prepared_mask: u64, active_mask: u64, registry_version: u64, bump }`. Bitet 0..=55 të vlefshme; 56..=63 duhet të jenë zero dhe refuzohen, nuk maskohen.
- Invariantet: pa bite të rezervuara; `active_mask ⊆ prepared_mask`; numri aktiv vetëm me `count_ones()`.
- `id < 56` kontrollohet përpara çdo shift-i; ID 56..=255 kthejnë `ShardIdOutOfRange` pa panic dhe pa ndryshim gjendjeje.
- Çdo tranzicion llogarit gjendjen e re plotësisht dhe e shkruan vetëm pasi kalojnë të gjitha kontrollet; `registry_version` rritet me `checked_add`.
- Tregu: `Initializing → Open`, `Open ↔ Paused`. Shard-i: `Prepared → Active`.
- Para hapjes lejohen vetëm `prepare` për ID 0..=7. `finalize_initial` kërkon `prepared_mask == 0xff`, `active_mask == 0` dhe `ready_mask == 0xff` të llogaritur nga rivlerësimi i llogarive në të njëjtin transaksion; vendos `active_mask = 0xff` dhe `Open` së bashku.
- Pas hapjes: `prepare` për çdo ID të papërdorur < 56; `activate` kërkon `Open`, prepared, jo aktiv dhe readiness aktuale. Aktivizimi i dyfishtë është gabim, jo no-op.
- `Paused` bllokon prepare/activate/swap/add liquidity; **nuk bllokon kurrë** withdraw/collect të LP-ve.
- Jashtë v1: deactivation, ripërdorim ID-sh, shrink, pauzë për shard të veçantë.

## Routing

`select_by_hash(mask, h)` = biti aktiv me rank `h % count_ones(mask)`; refuzon maskë zero. `hash & 7` është i barabartë me këtë vetëm kur maska është prefiksi `0xff`; `hash & 55` arrin vetëm 32 ID dhe është i ndaluar. Routing-u nuk është kufi sigurie: programi verifikon ID, bit, status dhe gjeneratën.

## Alternativat

- `active_count` i ruajtur: refuzuar, mund të dalë jashtë sinkronizimit.
- Maskë `u128` ose vektor: e panevojshme për 56 dhe rrit madhësinë/CU.
- `paused_mask` për pauzë selektive: e mundshme më vonë, kërkon ndryshim skeme dhe migrim; vendim i hapur në ADR-0004.

## Evidenca

`cargo test -p sharded-amm`: 23 teste, përfshirë çdo vlerë `u8` të ID-së, maskat 0/0xff/bit 55/plotë/sparse, aktivizim të dyfishtë, overflow të versionit, finalizim të pjesshëm dhe një model të rastësishëm (seed `0xabad1dea00000056`, 300 × 200 hapa). Kjo nuk provon ende instruksionet onchain, lidhjet e llogarive ose readiness reale (ENG-03/04).
