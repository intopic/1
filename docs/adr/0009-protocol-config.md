# ADR-0009: Rolet e protokollit, profilet e tregjeve dhe freeze authority

**Statusi:** ACCEPTED (gerfa, 29 tetor 2026, analiza për mainnet D-04, D-05, D-07). Implementuar dhe testuar.

## Konteksti

Para këtij vendimi, kush krijonte një treg bëhej admin i tij dhe zgjidhte vetë keeper-in, pauser-in dhe autoritetin e të ardhurave. Tregjet e memecoin-eve janë tregu kryesor, prandaj krijuesi mund të ishte kushdo. Rolet nuk mund të ndërroheshin, dhe një mint me freeze authority (që mund të ngrijë vault-et) pranohej pa kontroll.

## Vendimi

1. **Një `ProtocolConfig` global** (`["protocol"]`) mban rolet: admin, keeper, pauser dhe revenue_authority (treasury). E krijon vetëm upgrade authority e programit (`initialize_protocol`), një herë.
2. **Krijuesi nuk mban asnjë rol.** `initialize_market` është i hapur për këdo, por krijuesi vetëm paguan dhe zgjedh një profil. Rolet kopjohen nga protokolli. `MarketConfig::creator` ruhet vetëm si informacion.
3. **Profilet e lejuara.** Deri në 4 profile (spacing, tarifa, pjesa e protokollit, tarifa dinamike fillestare) i vendos admini me timelock 24 orë. Një treg merr parametrat vetëm nga profili.
4. **Rotacioni (D-04):**
   - `propose_roles` dhe pas 24 orësh `apply_roles`.
   - Një admin i ri bëhet vetëm `pending_admin` dhe e merr rolin me nënshkrimin e vet (`accept_admin`).
   - Admini mund ta heqë pauser-in menjëherë (`remove_pauser`).
   - Çdo ndryshim rrit `roles_version`. Çdo instruksion i privilegjuar i tregut kërkon `market.roles_version == protocol.roles_version`, kështu që çelësat e vjetër humbasin fuqinë në të gjitha tregjet njëherësh.
   - `sync_market_roles` është i hapur për këdo. Ai kopjon rolet aktuale në një treg, pa zgjedhur asnjë vlerë.
   - Slot-i i rebalancer-it ku ishte keeper-i i vjetër kalon te keeper-i i ri.
5. **Freeze authority (D-07):**
   - Një mint me freeze authority refuzohet, përveç mint-eve në `freeze_allowlist` (stablecoin-et e rregulluara si USDC dhe USDT), e cila ndryshohet me timelock.
   - Mint authority nuk refuzohet, por ndez një flamur rreziku (`risk_flags`) në treg dhe në event.
6. **Profilet e nisjes** (`ProtocolSettings::launch()`, 16% protokolli):

   | Profili | Spacing | Tarifa bazë | Tarifa dinamike |
   |---|---:|---:|---|
   | 0: stablecoin | 1 | 0,01% | fikur |
   | 1: SOL/USDC dhe majors | 10 | 0,05% | deri në 0,30% |
   | 2: memecoin | 200 | 1% | deri në 2% |

   Lista e freeze allowlist nis bosh; deployer-i shton mint-et reale të USDC/USDT.
7. **Tarifa e krijimit:** `creation_fee_lamports` ekziston në settings, që layout-i të mos ndryshojë më vonë. Ngarkimi i saj implementohet në fazën finalize.

## Pasojat

- Swap-et dhe operacionet e LP-ve nuk e lexojnë `ProtocolConfig`. Daljet e LP-ve nuk varen kurrë nga rolet: testi `protocol_roles` heq likuiditet dhe bën swap ndërkohë që rolet e tregut janë të vjetruara.
- Pas një rotacioni, çdo treg duhet sinkronizuar para veprimit të radhës së privilegjuar. Kushdo mund ta bëjë këtë, edhe në të njëjtin transaksion me veprimin.
- `MarketConfig` u rrit nga 643 në 680 bajt. `ProtocolConfig` ka 905 bajt. Kjo ndodhi para çdo deploy-i, prandaj `SCHEMA_VERSION` mbetet 1.

## Testet

- `state::protocol_config::tests`: timelock-u, pranimi me dy hapa, heqja e pauser-it dhe validimi i settings.
- `tests/svm/tests/protocol_roles.rs`:
  - vetëm upgrade authority e krijon protokollin;
  - krijuesi pa role;
  - rotacioni me `StaleRoles` dhe sinkronizimi;
  - heqja e menjëhershme e pauser-it;
  - freeze authority me listë.
- `eng03_accounts`: vetëm profilet e lejuara pranohen, dhe profilet jashtë kufijve refuzohen që në propozim.
