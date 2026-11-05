"""Ensure the benchmark rejects valid proofs bound to the wrong request."""
import copy
import datetime as dt
import hashlib
import unittest

import benchmark_parallel as bench


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        raw = bytes(range(32))
        self.client = {"wallet_bytes": raw, "wallet": bench.b58(raw), "kind": "balance",
                       "attested_at": 1234, "started_day": dt.date(2026, 11, 5)}
        claim = int.from_bytes(hashlib.sha256(b"balance").digest()[:8], "big")
        inputs = [17, 1234, claim, 42, 55, 10000, (1 << 64) - 1, 66,
                  int.from_bytes(raw[:16], "big"), int.from_bytes(raw[16:], "big")]
        self.proof = {"backend": "circom-rapidsnark", "wallet": self.client["wallet"],
                      "public_inputs": [hex(value) for value in inputs], "nullifier": hex(42)}

    def validate(self, proof):
        bench.validate_binding(proof, self.client, 66, 55, dt.date(2026, 11, 5))

    def test_correct_request_and_swapped_bindings(self):
        self.validate(self.proof)
        for index in (1, 2, 4, 5, 6, 7, 8, 9):
            with self.subTest(public_input=index):
                bad = copy.deepcopy(self.proof)
                bad["public_inputs"][index] = hex(int(bad["public_inputs"][index], 16) + 1)
                with self.assertRaises(ValueError):
                    self.validate(bad)
        for field, value in (("wallet", "another-user"), ("backend", "arkworks-dev"), ("nullifier", "0x43")):
            with self.subTest(field=field):
                bad = dict(self.proof, **{field: value})
                with self.assertRaises(ValueError):
                    self.validate(bad)

    def test_age_range_and_midnight(self):
        self.client["kind"] = "age"
        self.proof["public_inputs"][2] = hex(int.from_bytes(hashlib.sha256(b"date_of_birth").digest()[:8], "big"))
        self.proof["public_inputs"][5] = hex(19000101)
        self.proof["public_inputs"][6] = hex(20081105)
        self.validate(self.proof)
        self.proof["public_inputs"][6] = hex(20081106)
        bench.validate_binding(self.proof, self.client, 66, 55, dt.date(2026, 11, 6))
        with self.assertRaises(ValueError):
            self.validate(self.proof)
        self.assertEqual(bench.age_cutoff(dt.date(2024, 2, 29)), 20060228)

    def test_percentiles_include_tail_for_small_batches(self):
        self.assertIsNone(bench.percentile([], .95))
        self.assertEqual(bench.percentile(list(range(1, 17)), .95), 16)
        self.assertEqual(bench.percentile(list(range(1, 129)), .95), 122)
        self.assertEqual(bench.b58(bytes(32)), "1" * 32)


if __name__ == "__main__":
    unittest.main()
