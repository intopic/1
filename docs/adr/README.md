# Architecture Decision Records

Çdo vendim që ndikon matematikën, custody-n, ABI-në ose rolet regjistrohet këtu përpara kodit që e zbaton. Statuset: `PROPOSED` (pret rishikim), `ACCEPTED`, `OPEN` (kërkon vendim produkti), `SUPERSEDED`.

| ADR | Tema | Statusi |
|---|---|---|
| [0001](0001-numeric-domain.md) | Domain numerik Q64.64, rrumbullakimi, 512-bit `mul_div`, fee growth modular | PROPOSED |
| [0002](0002-registry-lifecycle.md) | Bitmask `u64`, lifecycle i tregut/shard-eve, politika e daljes | PROPOSED (implementuar si logjikë host, ENG-02) |
| [0003](0003-provenance-and-licensing.md) | Provenance, Raydium/Orca, `upstream-lock.json` | ACCEPTED |
| [0009](0009-protocol-config.md) | `ProtocolConfig`: rolet vetëm të protokollit, rotacion me timelock dhe pranim me dy hapa, profilet e lejuara, refuzimi i freeze authority me listë për stablecoin-at | ACCEPTED (implementuar dhe testuar) |
| [0004](0004-open-product-decisions.md) | Konfigurimi eksperimental: WSOL/USDC, tarifat 0,05–0,30%, spacing 10/60, rolet | ACCEPTED (eksperimental); disa pika OPEN |
| [0005](0005-dynamic-fee-per-shard.md) | Tarifa dinamike për shard: gjendja në `ShardState`, parametrat me timelock, 2% maksimum (0,80% deri më 29 tetor 2026) | PROPOSED (implementuar dhe simuluar) |
| [0006](0006-open-rebalance-pair.md) | `rebalance_pair` pa kapital, vetëm për rebalancer-at e listuar (timelock 24h); bit-e pauze për swap, depozita, rebalancim | PROPOSED (implementuar dhe testuar) |
| [0007](0007-threshold-router.md) | Router off-chain me prag: 1 shard për urdhra të vegjël, pak shard-e për të mëdhenjtë; `find_rebalance` | PROPOSED (implementuar dhe testuar) |
| [0008](0008-non-negotiable-rules.md) | Tetë rregullat e panegociueshme; prova e thellësisë në të dy drejtimet (`verify_depth`); pretendime vetëm me kapital total të barabartë | ACCEPTED |
