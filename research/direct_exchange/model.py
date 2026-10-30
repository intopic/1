"""Research model only: no cryptographic verification, RPC, or production use.

Balances are integer base units. The authorization set stands in for verified
signatures; receipt keys represent separate per-owner/per-nonce PDA accounts,
not a shared onchain order registry. Fee-payer locks are modeled, SOL fees not.
"""
from dataclasses import dataclass, replace
from itertools import combinations
from functools import lru_cache
import math
import random

U64_MAX = (1 << 64) - 1
DOMAIN = "direct-exchange-research:v1"


class Rejected(ValueError):
    pass


@dataclass(frozen=True)
class Account:
    owner: str
    mint: str
    amount: int


@dataclass(frozen=True)
class Order:
    owner: str
    nonce: int
    source: str
    destination: str
    sell_mint: str
    buy_mint: str
    sell_amount: int
    min_receive: int
    expiry: int = 100
    domain: str = DOMAIN

    @property
    def receipt(self):
        return f"receipt:{self.domain}:{self.owner}:{self.nonce}"


@dataclass(frozen=True)
class Exchange:
    name: str
    first: Order
    second: Order
    payer: str
    authorized: frozenset[str]

    def footprint(self):
        orders = (self.first, self.second)
        reads = {"policy:v1", "program:spl-token"}
        writes = {self.payer}
        for order in orders:
            reads.update((order.owner, f"mint:{order.sell_mint}", f"mint:{order.buy_mint}"))
            writes.update((order.source, order.destination, order.receipt))
        return Candidate(self.name, frozenset(reads), frozenset(writes))


@dataclass(frozen=True)
class Candidate:
    name: str
    reads: frozenset[str]
    writes: frozenset[str]


@dataclass
class State:
    accounts: dict[str, Account]
    consumed: set[str]

    def clone(self):
        return State(self.accounts.copy(), self.consumed.copy())


def totals(state):
    result = {}
    for account in state.accounts.values():
        result[account.mint] = result.get(account.mint, 0) + account.amount
    return result


def uint(value, positive=False):
    return type(value) is int and (1 if positive else 0) <= value <= U64_MAX


def settle(state, exchange, now=0, fail_after_first=False):
    """Full-fill bilateral exchange. Return new state only after both legs pass."""
    if not uint(now):
        raise Rejected("invalid clock")
    a, b = exchange.first, exchange.second
    if a.owner == b.owner:
        raise Rejected("self exchange excluded from this model")
    if (a.sell_mint, a.buy_mint) != (b.buy_mint, b.sell_mint):
        raise Rejected("incompatible mints")
    if a.sell_mint == a.buy_mint:
        raise Rejected("same mint")
    for order in (a, b):
        if order.domain != DOMAIN:
            raise Rejected("wrong authorization domain")
        if order.owner not in exchange.authorized or exchange.payer not in exchange.authorized:
            raise Rejected("missing model authorization")
        if not uint(order.nonce) or not uint(order.expiry) or order.expiry < now:
            raise Rejected("nonce or expiry")
        if not uint(order.sell_amount, True) or not uint(order.min_receive, True):
            raise Rejected("invalid quantity")
        if order.receipt in state.consumed:
            raise Rejected("already consumed")
        source = state.accounts.get(order.source)
        dest = state.accounts.get(order.destination)
        if source is None or dest is None:
            raise Rejected("missing token account")
        if not uint(source.amount) or not uint(dest.amount):
            raise Rejected("invalid token balance")
        if source.owner != order.owner or dest.owner != order.owner:
            raise Rejected("wrong account owner")
        if source.mint != order.sell_mint or dest.mint != order.buy_mint:
            raise Rejected("wrong token account mint")
    if b.sell_amount < a.min_receive or a.sell_amount < b.min_receive:
        raise Rejected("limit price")
    draft = state.clone()
    for number, (order, other) in enumerate(((a, b), (b, a))):
        source = draft.accounts[order.source]
        destination = draft.accounts[other.destination]
        if source.amount < order.sell_amount:
            raise Rejected("insufficient funds")
        if destination.amount + order.sell_amount > U64_MAX:
            raise Rejected("destination overflow")
        draft.accounts[order.source] = replace(source, amount=source.amount - order.sell_amount)
        draft.accounts[other.destination] = replace(destination, amount=destination.amount + order.sell_amount)
        if number == 0 and fail_after_first:
            raise Rejected("injected second leg failure")
    draft.consumed.update((a.receipt, b.receipt))
    if totals(draft) != totals(state):
        raise AssertionError("token conservation")
    return draft


def conflicts(a, b):
    return bool(a.writes & (b.reads | b.writes) or b.writes & (a.reads | a.writes))


def graph(candidates):
    adjacency = [set() for _ in candidates]
    for i, j in combinations(range(len(candidates)), 2):
        if conflicts(candidates[i], candidates[j]):
            adjacency[i].add(j)
            adjacency[j].add(i)
    return adjacency


def independent(indices, adjacency):
    if len(indices) != len(set(indices)):
        return False
    if any(type(i) is not int or not 0 <= i < len(adjacency) for i in indices):
        return False
    return all(j not in adjacency[i] for i, j in combinations(indices, 2))


def capacity_projection(indices, costs, budget):
    """Optional one-resource admission cap, e.g. a conservative CU estimate.

    A model cap is not a replacement for runtime-enforced compute limits.
    """
    if not uint(budget) or any(not uint(c) for c in costs):
        raise ValueError("invalid resource budget")
    if len(indices) != len(set(indices)) or any(type(i) is not int or not 0 <= i < len(costs) for i in indices):
        raise ValueError("invalid candidate indices")
    result, used = [], 0
    for i in indices:
        if used + costs[i] <= budget:
            result.append(i)
            used += costs[i]
    return result


def first_fit(adjacency, priorities=None):
    priorities = range(len(adjacency)) if priorities is None else priorities
    if len(priorities) != len(adjacency) or set(priorities) != set(range(len(adjacency))):
        raise ValueError("priorities must be a permutation of candidate indices")
    result, excluded = [], set()
    for i in priorities:
        if i not in excluded:
            result.append(i)
            excluded.update(adjacency[i])
    return result


def degree_greedy(adjacency):
    remaining, result = set(range(len(adjacency))), []
    while remaining:
        i = min(remaining, key=lambda x: (len(adjacency[x] & remaining), x))
        result.append(i)
        remaining.difference_update(adjacency[i] | {i})
    return result


def local_inhibition(adjacency, seed=0):
    """Local winner/inhibit-neighbors MIS heuristic; no optimality claim."""
    rng = random.Random(seed)
    remaining, result = set(range(len(adjacency))), []
    while remaining:
        ranks = {i: (rng.getrandbits(64), i) for i in sorted(remaining)}
        winners = [i for i in sorted(remaining)
                   if all(ranks[i] > ranks[j] for j in adjacency[i] & remaining)]
        assert winners
        result.extend(winners)
        for i in winners:
            remaining.difference_update(adjacency[i] | {i})
    assert independent(result, adjacency)
    return result


def inhibitory_scores(adjacency, seed=0, steps=200, drive=4.0, inhibition=4.0):
    """Exploratory lateral-inhibition ODE, Euler dt=.05; not an allocator.

    dx_i/dt = sigmoid(drive - inhibition*sum_j A_ij*x_j) - x_i.
    These coefficients are experimental, not biologically calibrated.
    """
    rng = random.Random(seed)
    x = [rng.random() for _ in adjacency]
    for _ in range(steps):
        x = [value + .05 * (1 / (1 + math.exp(max(-700, min(700,
             -(drive - inhibition * sum(x[j] for j in adjacency[i])))))) - value)
             for i, value in enumerate(x)]
    return x


def bio_projected(adjacency, seed=0):
    scores = inhibitory_scores(adjacency, seed)
    # The discrete projection is essential: continuous concentrations are not
    # authority to spend funds or to bypass account locks.
    return first_fit(adjacency, sorted(range(len(scores)), key=lambda i: (-scores[i], i)))


def competitive_scores(adjacency, seed=0, initial=None, steps=400, tau=2.0):
    """Mooij-Kryven competitive-network equation, finite Euler experiment.

    dx_i/dt = x_i*(1-x_i-tau*sum_j A_ij*x_j); dt=.025.
    Floating-point integration is for scheduler ranking only. No convergence
    theorem is claimed for this finite discretization or these workloads.
    """
    rng = random.Random(seed)
    x = list(initial) if initial is not None else [.1 + .8 * rng.random() for _ in adjacency]
    for _ in range(steps):
        x = [max(0.0, min(1.0, value + .025 * value *
             (1 - value - tau * sum(x[j] for j in adjacency[i]))))
             for i, value in enumerate(x)]
    return x


def competitive_projected(adjacency, seed=0):
    scores = competitive_scores(adjacency, seed)
    return first_fit(adjacency, sorted(range(len(scores)), key=lambda i: (-scores[i], i)))


def exact_maximum(adjacency):
    """Exact small-instance reference; exponential, not a production scheduler."""
    if len(adjacency) > 24:
        raise ValueError("exact reference limited to 24 vertices")
    masks = [sum(1 << j for j in neighbors) for neighbors in adjacency]

    @lru_cache(None)
    def solve(mask):
        if not mask:
            return ()
        vertices = [i for i in range(len(adjacency)) if mask & (1 << i)]
        i = max(vertices, key=lambda j: (masks[j] & mask).bit_count())
        without = solve(mask & ~(1 << i))
        including = (i,) + solve(mask & ~(masks[i] | (1 << i)))
        return including if len(including) > len(without) else without

    return list(solve((1 << len(adjacency)) - 1))


def fixture(n=8, shared_payer=False):
    accounts, exchanges = {}, []
    for i in range(n):
        a, b = f"wallet:a{i}", f"wallet:b{i}"
        for owner in (a, b):
            for mint in ("A", "B"):
                accounts[f"{owner}:{mint}"] = Account(owner, mint, 1000)
        first = Order(a, 0, f"{a}:A", f"{a}:B", "A", "B", 10, 20)
        second = Order(b, 0, f"{b}:B", f"{b}:A", "B", "A", 20, 10)
        payer = "wallet:sponsor" if shared_payer else a
        exchanges.append(Exchange(f"exchange:{i}", first, second, payer,
                                  frozenset((a, b, payer))))
    return State(accounts, set()), exchanges
