# Direct exchange: mathematical research specification

Status: executable model and wire/signature experiment. No deployed protocol,
custom Solana settlement program, runtime speedup, or new security theorem.

## 1. Exact bilateral exchange

For token account a, balance b_a is an integer in [0, 2^64-1]. Mint(a)
identifies its token; owner(a) identifies its transfer authority in the model.
Accounts remain token-specific: surplus B never compensates missing A.

An exact-sell order contains domain, owner, nonce, source, destination,
sell_mint, buy_mint, sell_amount x, minimum receive y, and expiry. For a full
fill, debit d=x and credit c>=y. Partial fills are deliberately not implemented.
Their necessary price condition would be c*x>=d*y using wide integer products.

A candidate pairs two compatible orders. Each sells its exact x and receives
the other's exact x, subject to both minima. There is no AMM curve, external
price oracle, reserve pool, LP, loan or liquidation in this experiment.
Matching must find a funded counterparty: no formula creates absent assets.

For every traded token t, including any explicit fee recipients:

    sum_a Delta[b_a] over accounts with Mint(a)=t = 0
    0 <= b_a + Delta[b_a] <= 2^64-1

Protocol token fees are zero in this prototype. SOL network fees are excluded
from the model's token ledger, not assumed to be zero on Solana.

Full-fill receipt key = (domain, owner, nonce); it is consumed at most once.
The Python consumed-set models separate onchain receipt accounts, NOT one
shared writable registry. Receipt funding, closing, cancellation and upgrades
are not implemented. The model's authorization set is a test assumption, not
signature verification or token-transfer authority.

Atomicity in model.py: work on a draft copy; return it only after both
transfers pass. Exceptions leave balances and receipts unchanged. This tests
our model, not the Solana runtime's implementation of rollback.

## 2. Solana conflict condition

For candidate i derive complete requested read/write sets R_i,W_i. Include
user token accounts, payer, receipts and every CPI account. Program IDs,
mint accounts and static policy may be shared read-only.

    A_ij = 1 iff W_i intersects (R_j union W_j)
                  or W_j intersects (R_i union W_i)

No-conflict selection with unit value:

    maximize sum_i z_i
    z_i in {0,1}; z_i+z_j<=1 whenever A_ij=1

This is maximum independent set, a known hard problem. The exact reference
is capped at 24 candidates. All benchmark objective weights equal one.
The greedy/biological algorithms return maximal sets, not guaranteed maximum.
General economic weighting and starvation resistance remain unimplemented.

Independence is necessary for this account-lock parallelism experiment, not
a guarantee of execution overlap or throughput: validator cores, CU, slot,
block/account cost budgets, networking and inclusion also constrain execution.

Global capacity requires an additional condition sum_i c_pi*z_i<=B_p.
Example: each candidate costs 4, budget10. Every pair costs8 but three cost12.
capacity_projection checks a one-resource cap; costs here are abstract, not
measured CU or validator limits. Chain budgets must be checked separately.

## 3. Conditional compositional proof

Assumptions: (a) complete and accurate footprints, (b) each candidate valid
on initial state, (c) no hidden writes/hooks and unchanged shared policy,
(d) bilateral assets/authorities are verified, (e) no unsupported token behavior.

For a selected pair with A_ij=0, neither transition modifies anything the
other reads or writes. Its validation inputs therefore remain unchanged.
Their balance writes affect disjoint coordinates and receipts are distinct.
Consequently T_i(T_j(S))=T_j(T_i(S)) and both retain their validity. Induction
extends this to any finite selected set. Conservation follows by summing
zero-net per-token transfer vectors; disjoint source debits prevent double
spend within that set. Failed candidates contribute no committed delta.

This conditional argument is not machine-checked verification. The tests
exhaust all24 permutations for four disjoint exchanges, compare112 generated
graphs to exact optima, and exercise500 random integer bilateral fills.

When two candidates spend the same source account, the condition fails.
Even with enough funds to execute both sequentially, Solana's account lock
remains shared. Distinct nonces do not manufacture independent funds.

## 4. Biological translation

Chemical/Petri representation: m'=m+(Post-Pre)z, Pre*z<=m. Columns represent
settlements; per-token conservation vectors l_t satisfy l_t^T(Post-Pre)=0.
No transfer can consume an expected future credit as existing funding in the
conservative independent-wave model. Atomic cyclic settlement is future work.

Competitive-species equation from Mooij-Kryven (2024):

    dx_i/dt = x_i*(1-x_i-tau*sum_j A_ij*x_j), tau>1

Here x_i is scheduler activation, NOT a token balance. The paper's almost-
everywhere asymptotic statement does not establish convergence after400
Euler steps. Our experiment uses dt=.025, tau2, seed0, then discrete greedy
projection; no claim to reproduce its full continuation algorithm or results.

An additional exploratory sigmoid inhibition score is:

    dx_i/dt = sigmoid(4-4*sum_j A_ij*x_j)-x_i

It uses dt=.05,200 steps; parameters are not biologically calibrated.
Both score functions only rank candidates. A mandatory discrete projection
rejects every conflicting candidate, regardless of its continuous score.

Exact counterexample: two connected nodes, tau2, x_1=x_2=1/3. This is a
nonbinary equilibrium. Threshold .25 selects BOTH (unsafe); threshold .5
selects NONE. Numerical concentration is never spending authorization.

Local winner/inhibit-neighbors election is also tested. It is related to
known biology-inspired maximal independent set work, not a new discovery.
On an8-leaf star it sometimes selects only the center instead of all8 leaves.

## 5. Scope of protocol_probe.py

Constructs real legacy Solana messages with two classic SPL TransferChecked
instructions and actual Ed25519 transaction signatures from both owners.
Derives requested R/W sets from serialized message headers, not a hand-labeled
graph. Rejects tampered amount/missing signatures using solders verification.

It does NOT create accounts, verify token balances or execute SPL instructions.
It uses an all-zero placeholder blockhash and never submits transactions.
It is NOT an implementation of the model's domain/nonce/expiry order protocol.

## 6. Main invariants not yet proven or implemented

- Cryptographic binding of persistent orders, expiry and cancellation onchain.
- Receipt lifecycle/rent and replay after account closure.
- Adversarial matching, fairness, Sybil resistance or incentive compatibility.
- Token-2022 hooks, fee-bearing transfers and unsupported mint configurations.
- Live funding changes between selection and settlement; stale quote economics.
- Scheduler overlap, landing success, end-to-end latency and8x speedup.
- Production contract correctness or superior security to current exchanges.
