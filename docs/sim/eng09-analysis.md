# ENG-09: what the simulation says, and what it does not

Raw numbers: [eng09-results.md](eng09-results.md). Code and model:
`crates/sim/src/main.rs`. Both venues run the production math (the
`sharded_amm::engine` CLMM and the `amm_model::cpmm` baseline), so the
rounding and fee rules are the real ones. The market model around them is a
simple one and is the main limit on what the numbers mean.

## Findings

1. **Traders pay much less on the CLMM than on the CPMM with the same capital.**
   With $1M of LP capital, uninformed traders paid about 10 to 35 bps on a
   ±5% CLMM range and about 175 to 197 bps on the CPMM. Most of that
   difference is price impact. Concentrating the capital is what gives the
   better prices.

2. **Tick spacing 10 and 60 behave almost the same for this pair.**
   At equal fee and range width the results differ by less than the noise
   across seeds. Spacing 10 gives slightly lower trader cost because the range
   edges sit closer to the target width. Spacing 60 costs fewer tick crossings
   (compute units) for the same price move. Neither is a clear win on these
   numbers. The choice can rest on CU cost and on how finely LPs want to place
   ranges.

3. **Fee tier sets the split between LPs and traders, as expected.**
   Going from 5 to 30 bps raises LP fee income about 4x and raises trader cost
   by roughly the fee difference. With volatility near 4% a day and this
   order flow, all three tiers were LP-positive against HODL on average.
   The ±5% ranges have a much wider spread across seeds, because the price
   sometimes leaves the range.

4. **Sharding splits liquidity, and routing decides whether that hurts traders.**
   - With 8 equal shards and **best-single-shard routing**, trader cost rises
     sharply. For example, it went from 21 bps to 98 bps at 5 bps with ±20%
     ranges, because each order sees only 1/8 of the liquidity. LPs gain the
     same amount, because the extra cost is price impact they capture.
   - With the order **split evenly across all 8 shards**, the results are
     identical to a single pool with the same total liquidity.
   - Conclusion: the SDK router must split large orders across shards.
     Sending each order to one shard is not acceptable for traders.
   - A transaction that touches 8 shards needs about 8 × 33k CU base
     (measured), plus about 9k per tick crossing. It needs an address lookup
     table to fit the 1232-byte packet. The lookup table does **not** raise
     the 64-account lock limit: 8 shards with 3 tick arrays each plus
     ComputeBudget is exactly 64 accounts, so anything extra (a SOL wrap, an
     ATA creation) needs fewer tick arrays or fewer shards. See
     [tx-budget.md](../tx-budget.md). (Corrected after audit F-04; an earlier
     version said the lookup table solves the account limit.) In practice the router
     should split across the fewest shards that bring the price impact down.
     Smaller orders can go to one shard.

5. **The CPMM's higher LP PnL here is a modeling artifact.**
   Uninformed traders in the model do not react to cost, so the CPMM's 175+
   bps of impact all flows to its LPs. Real traders and aggregators would
   route away from a venue that charges 10x more.

## What the model leaves out

- Trader demand that reacts to price.
- Gas and priority fees.
- MEV and sandwiching.
- JIT liquidity.
- LP rebalancing.
- Competing venues.
- Fat-tailed or trending price paths.
- Arbitrage latency: the arbitrageur here always moves first in each step.

These results are a sanity check of the design and of the fee and spacing
choices. They are not a forecast of revenue or LP returns.

## Recommendation for the devnet experiment (ADR-0004)

- Keep **30 bps and 5 bps** as the two arms. 10 bps sits between them on
  every metric.
- Use **spacing 60** for the first shards, since it costs less compute and
  gives the same economics here. Keep spacing 10 as a variant.
- Treat **split routing across shards** as a requirement for the client and
  SDK work.
