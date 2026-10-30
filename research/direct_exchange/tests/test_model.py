import itertools
import random
import unittest
from dataclasses import replace

from model import (Account, Candidate, Rejected, U64_MAX, bio_projected,
                   capacity_projection, competitive_projected, competitive_scores, conflicts,
                   degree_greedy, exact_maximum, first_fit, fixture,
                   graph, independent, inhibitory_scores, local_inhibition,
                   settle, totals)


class SettlementTests(unittest.TestCase):
    def test_eight_disjoint_exchanges_preserve_all_tokens(self):
        state, txs = fixture()
        original = totals(state)
        self.assertEqual(len(exact_maximum(graph([t.footprint() for t in txs]))), 8)
        for tx in txs:
            state = settle(state, tx)
        self.assertEqual(totals(state), original)
        self.assertEqual(len(state.consumed), 16)

    def test_every_ordering_of_four_disjoint_exchanges_agrees(self):
        initial, txs = fixture(4)
        results = []
        for ordering in itertools.permutations(txs):
            state = initial
            for tx in ordering:
                state = settle(state, tx)
            results.append(state)
        self.assertTrue(all(s == results[0] for s in results))

    def reject_unchanged(self, state, tx, **kwargs):
        before = state.clone()
        with self.assertRaises(Rejected):
            settle(state, tx, **kwargs)
        self.assertEqual(state, before)

    def test_atomic_rollback_after_first_transfer(self):
        state, txs = fixture(1)
        self.reject_unchanged(state, txs[0], fail_after_first=True)
        self.assertEqual(len(settle(state, txs[0]).consumed), 2)

    def test_second_leg_insufficient_funds_rolls_back_first(self):
        state, txs = fixture(1)
        key = txs[0].second.source
        state.accounts[key] = replace(state.accounts[key], amount=19)
        self.reject_unchanged(state, txs[0])

    def test_replay(self):
        state, txs = fixture(1)
        state = settle(state, txs[0])
        self.reject_unchanged(state, txs[0])

    def test_changed_order_same_nonce_cannot_replay(self):
        state, txs = fixture(1)
        tx = txs[0]
        state = settle(state, tx)
        self.reject_unchanged(state, replace(tx, first=replace(tx.first, sell_amount=11)))

    def test_expiry_and_boundary(self):
        state, txs = fixture(1)
        self.reject_unchanged(state, txs[0], now=101)
        self.assertEqual(len(settle(state, txs[0], now=100).consumed), 2)

    def test_invalid_clock_is_rejected(self):
        state, txs = fixture(1)
        for now in (-1, True, float("nan"), U64_MAX + 1):
            self.reject_unchanged(state, txs[0], now=now)

    def test_missing_authorization(self):
        state, txs = fixture(1)
        self.reject_unchanged(state, replace(txs[0], authorized=frozenset()))

    def test_wrong_domain(self):
        state, txs = fixture(1)
        self.reject_unchanged(state, replace(txs[0], first=replace(txs[0].first, domain="other-chain")))

    def test_limit_violation(self):
        state, txs = fixture(1)
        self.reject_unchanged(state, replace(txs[0], first=replace(txs[0].first, min_receive=21)))

    def test_wrong_source_owner(self):
        state, txs = fixture(1)
        key = txs[0].first.source
        state.accounts[key] = replace(state.accounts[key], owner="thief")
        self.reject_unchanged(state, txs[0])

    def test_wrong_destination_owner(self):
        state, txs = fixture(1)
        key = txs[0].first.destination
        state.accounts[key] = replace(state.accounts[key], owner="thief")
        self.reject_unchanged(state, txs[0])

    def test_wrong_mint(self):
        state, txs = fixture(1)
        key = txs[0].second.destination
        state.accounts[key] = replace(state.accounts[key], mint="C")
        self.reject_unchanged(state, txs[0])

    def test_invalid_quantities(self):
        state, txs = fixture(1)
        for amount in (0, -1, U64_MAX + 1, True):
            with self.subTest(amount=amount):
                self.reject_unchanged(state, replace(txs[0], first=replace(txs[0].first, sell_amount=amount)))

    def test_overflow_in_second_transfer_is_atomic(self):
        state, txs = fixture(1)
        key = txs[0].first.destination
        state.accounts[key] = replace(state.accounts[key], amount=U64_MAX - 19)
        self.reject_unchanged(state, txs[0])

    def test_invalid_input_balance_is_rejected(self):
        for amount in (-1, U64_MAX + 1, True):
            state, txs = fixture(1)
            key = txs[0].first.source
            state.accounts[key] = replace(state.accounts[key], amount=amount)
            self.reject_unchanged(state, txs[0])

    def test_same_funds_distinct_nonces_cannot_double_spend(self):
        state, txs = fixture(1)
        tx = txs[0]
        for key, amount in ((tx.first.source, 10), (tx.second.source, 20)):
            state.accounts[key] = replace(state.accounts[key], amount=amount)
        another = replace(tx, name="other", first=replace(tx.first, nonce=1),
                          second=replace(tx.second, nonce=1))
        self.assertTrue(conflicts(tx.footprint(), another.footprint()))
        state = settle(state, tx)
        self.reject_unchanged(state, another)

    def test_random_integer_conservation(self):
        rng = random.Random(4282)
        for _ in range(500):
            state, txs = fixture(1)
            a, b = rng.randrange(1, 1001), rng.randrange(1, 1001)
            tx = replace(txs[0], first=replace(txs[0].first, sell_amount=a, min_receive=b),
                         second=replace(txs[0].second, sell_amount=b, min_receive=a))
            self.assertEqual(totals(settle(state, tx)), totals(state))


class SchedulingTests(unittest.TestCase):
    def test_duplicate_or_invalid_projection_indices_rejected(self):
        self.assertFalse(independent([0, 0], [set()]))
        for priorities in ([0, 0], [1], [], [-1]):
            with self.assertRaises(ValueError):
                first_fit([set()], priorities)
        for indices in ([0, 0], [-1], [1]):
            with self.assertRaises(ValueError):
                capacity_projection(indices, [1], 10)
    def test_pairwise_capacity_is_not_aggregate_capacity(self):
        self.assertLessEqual(4 + 4, 10)
        chosen = capacity_projection([0, 1, 2], [4, 4, 4], 10)
        self.assertEqual(chosen, [0, 1])
        self.assertEqual(sum([4, 4, 4][i] for i in chosen), 8)

    def test_capacity_rejects_invalid_parameters(self):
        for costs, budget in [([-1], 10), ([1], -1), ([True], 10)]:
            with self.assertRaises(ValueError):
                capacity_projection([0], costs, budget)
    def test_read_read_is_safe_but_read_write_is_not(self):
        a = Candidate("a", frozenset(("policy", "x")), frozenset(("a",)))
        b = Candidate("b", frozenset(("policy",)), frozenset(("b",)))
        c = Candidate("c", frozenset(), frozenset(("x",)))
        self.assertFalse(conflicts(a, b))
        self.assertTrue(conflicts(a, c))
        self.assertTrue(conflicts(c, a))

    def test_shared_payer_destroys_eight_way_independence(self):
        _, txs = fixture(8, shared_payer=True)
        self.assertEqual(len(exact_maximum(graph([t.footprint() for t in txs]))), 1)

    def test_shared_treasury_destroys_independence(self):
        _, txs = fixture()
        candidates = [replace(t.footprint(), writes=t.footprint().writes | {"treasury"}) for t in txs]
        self.assertEqual(len(exact_maximum(graph(candidates))), 1)

    def test_stale_hidden_metadata_must_not_be_trusted(self):
        _, txs = fixture()
        real = [replace(t.footprint(), writes=t.footprint().writes | {"shared"}) for t in txs]
        claimed = [t.footprint() for t in txs]
        selected = exact_maximum(graph(claimed))
        self.assertEqual(len(selected), 8)
        self.assertFalse(independent(selected, graph(real)))

    def test_first_fit_star_counterexample(self):
        adjacency = [set(range(1, 9))] + [{0} for _ in range(8)]
        self.assertEqual(len(first_fit(adjacency)), 1)
        self.assertEqual(len(degree_greedy(adjacency)), 8)
        self.assertEqual(len(exact_maximum(adjacency)), 8)

    def test_local_inhibition_is_not_maximum(self):
        adjacency = [set(range(1, 9))] + [{0} for _ in range(8)]
        sizes = {len(local_inhibition(adjacency, seed)) for seed in range(100)}
        self.assertEqual(sizes, {1, 8})

    def test_naive_ode_threshold_is_unsafe(self):
        adjacency = [{1}, {0}]
        # Weak inhibition permits both conflicting nodes to be highly active.
        scores = inhibitory_scores(adjacency, inhibition=.1)
        selected = [i for i, value in enumerate(scores) if value > .5]
        self.assertEqual(selected, [0, 1])
        self.assertFalse(independent(selected, adjacency))
        self.assertTrue(independent(bio_projected(adjacency), adjacency))

    def test_competitive_equation_symmetric_nonbinary_equilibrium(self):
        scores = competitive_scores([{1}, {0}], initial=[1 / 3, 1 / 3])
        self.assertEqual(scores, [1 / 3, 1 / 3])
        self.assertFalse(independent([i for i, x in enumerate(scores) if x > .25], [{1}, {0}]))
        self.assertEqual([i for i, x in enumerate(scores) if x > .5], [])

    def test_all_heuristics_safe_and_bounded_by_exact(self):
        rng = random.Random(9098)
        for n in range(1, 15):
            for _ in range(8):
                adjacency = [set() for _ in range(n)]
                p = rng.random()
                for i, j in itertools.combinations(range(n), 2):
                    if rng.random() < p:
                        adjacency[i].add(j)
                        adjacency[j].add(i)
                optimum = len(exact_maximum(adjacency))
                for fn in (first_fit, degree_greedy, local_inhibition, bio_projected, competitive_projected):
                    selected = fn(adjacency)
                    self.assertTrue(independent(selected, adjacency))
                    self.assertLessEqual(len(selected), optimum)
                    self.assertTrue(all(i in selected or adjacency[i] & set(selected) for i in range(n)))


if __name__ == "__main__":
    unittest.main()
