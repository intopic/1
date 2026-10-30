# Primary sources and semantic translation

Reviewed for this study on 2026-10-30. Sources establish existing methods;
none proves that our prototype is novel, secure or8x faster.

1. Solana, instruction/account metadata:
   https://solana.com/docs/core/instructions/instruction-structure
   Read/write permissions determine possible transaction independence.
2. Solana, atomic transactions:
   https://solana.com/docs/core/transactions
   All instructions commit or revert together; fees still apply on failure.
3. Solana, transaction pipeline:
   https://solana.com/docs/core/transactions/transaction-pipeline
   Signature checks, account loading, execution and commit are separate stages.
4. Mooij and Kryven, Finding Large Independent Sets in Networks Using
   Competitive Dynamics, 2024:
   https://arxiv.org/html/2409.01336v1
   Competitive Lotka-Volterra dynamics connect ecological survival to maximal
   independent sets. Asymptotic, almost-everywhere conditions matter. We test
   a finite simple LV discretization, not the full continuation implementation.
5. Afek et al., A Biological Solution to a Fundamental Distributed Computing
   Problem, Science2011; record:
   https://pubmed.ncbi.nlm.nih.gov/21233379/
   Related formal paper: https://arxiv.org/abs/1206.0150
   Lateral inhibition to maximal independent set is already a known connection.
6. What makes a reaction network chemical?, 2022:
   https://pmc.ncbi.nlm.nih.gov/articles/PMC9484159/
   Conservation/stoichiometric constraints inspire token accounting. Chemical
   species in scheduler equations do not represent mintable financial tokens.
7. Coordination Avoidance in Database Systems:
   https://arxiv.org/abs/1402.2237
   Invariant-confluence distinguishes operations safe without coordination
   from those that need coordination. It does not change Solana account locks.
8. CMU Problem Based Benchmark Suite, maximal independent set:
   https://www.cs.cmu.edu/~pbbs/benchmarks/maximalIndependentSet.html
   A maximal set is not necessarily a maximum set.

Translation: species/cells -> candidate exchanges; competing neighbors ->
RW conflicts; population/activity -> scheduling score; survival -> admission;
stoichiometric conservation -> per-token balance invariants. This is a
cross-domain hypothesis with known mathematical precedents, not evidence
of an undiscovered biological function that bypasses shared-state dependencies.
