# Architecture Decision Records

Çdo vendim që ndikon matematikën, custody-n, ABI-në ose rolet regjistrohet këtu përpara kodit që e zbaton. Statuset: `PROPOSED` (pret rishikim), `ACCEPTED`, `OPEN` (kërkon vendim produkti), `SUPERSEDED`.

| ADR | Tema | Statusi |
|---|---|---|
| [0001](0001-numeric-domain.md) | Domain numerik Q64.64, rrumbullakimi, 512-bit `mul_div`, fee growth modular | PROPOSED |
| [0002](0002-registry-lifecycle.md) | Bitmask `u64`, lifecycle i tregut/shard-eve, politika e daljes | PROPOSED (implementuar si logjikë host, ENG-02) |
| [0003](0003-provenance-and-licensing.md) | Provenance, Raydium/Orca, `upstream-lock.json` | ACCEPTED |
| [0004](0004-open-product-decisions.md) | Konfigurimi eksperimental: WSOL/USDC, tarifat 0,05–0,30%, spacing 10/60, rolet | ACCEPTED (eksperimental); disa pika OPEN |
