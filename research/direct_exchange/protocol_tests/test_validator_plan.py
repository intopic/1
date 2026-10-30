"""Check actual benchmark messages and genesis inputs without starting a node."""
import base64
import json
from pathlib import Path
import struct
import tempfile
import unittest

from solders.hash import Hash

from model import exact_maximum, graph
from protocol_probe import footprint
from validator_probe import TOKEN, build, fixture, verify_program_image


class ValidatorPlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name) / "accounts"
        self.mints, self.cases, self.negative, self.state, self.supplies = fixture(self.directory)

    def tearDown(self):
        self.temp.cleanup()

    def test_real_message_locks_and_equal_payloads(self):
        sizes = set()
        for name, pairs in self.cases.items():
            txs = [build(self.mints, pair, Hash.default()) for pair in pairs]
            for tx in txs:
                tx.verify()
                self.assertEqual(len(tx.signatures), 3)
                sizes.add(len(bytes(tx)))
            width = len(exact_maximum(graph([footprint(t, str(i)) for i, t in enumerate(txs)])))
            self.assertEqual(width, 8 if name == "independent" else 1)
        self.assertEqual(sizes, {584})

    def test_preloaded_supplies_and_token_accounts_are_consistent(self):
        images = {p.stem: json.loads(p.read_text())["account"] for p in self.directory.glob("*.json")}
        self.assertEqual(len(images), len(list(self.directory.glob("*.json"))))
        for mint, supply in self.supplies.items():
            data = base64.b64decode(images[mint]["data"][0])
            self.assertEqual(len(data), 82)
            self.assertEqual(struct.unpack_from("<Q", data, 36)[0], supply)
            self.assertEqual(data[44:46], bytes([6, 1]))
            self.assertEqual(sum(v["amount"] for v in self.state.values() if v["mint"] == mint), supply)
        for key, item in self.state.items():
            image = images[key]
            data = base64.b64decode(image["data"][0])
            self.assertEqual(image["owner"], str(TOKEN))
            self.assertEqual(len(data), 165)
            self.assertEqual(data[108], 1)
            self.assertEqual(struct.unpack_from("<Q", data, 64)[0], item["amount"])

    def test_negative_control_has_second_leg_insufficient_funds(self):
        a, b, payer, ax, ay, bx, by = self.negative
        self.assertGreaterEqual(self.state[str(ax)]["amount"], 10)
        self.assertEqual(self.state[str(by)]["amount"], 19)
        bad = build(self.mints, self.negative, Hash.default(), sell_b=1001)
        bad.verify()
        self.assertEqual(len(bad.signatures), 3)

    def test_programdata_pointer_is_not_treated_as_elf(self):
        from solders.pubkey import Pubkey
        owner = "BPFLoaderUpgradeab1e11111111111111111111111"
        address = Pubkey.new_unique()
        elf = b"\x7fELFtest-fixture"
        program = {"owner": owner, "executable": True,
                   "data": [base64.b64encode(struct.pack("<I", 2)+bytes(address)).decode(), "base64"]}
        metadata = struct.pack("<I", 3)+bytes(41)
        programdata = {"owner": owner, "executable": False,
                       "data": [base64.b64encode(metadata+elf+bytes(8)).decode(), "base64"]}
        result = verify_program_image(program, lambda key: programdata, elf)
        self.assertEqual(result["zero_padding_bytes"], 8)
        with self.assertRaises(AssertionError):
            verify_program_image(program, lambda key: programdata, b"\x7fELFchanged")
        sentinel = metadata[:12]+bytes([1])+bytes(32)
        programdata["data"][0] = base64.b64encode(sentinel+elf).decode()
        self.assertEqual(verify_program_image(program, lambda key: programdata, elf)["authority_encoding"],
                         "agave-default-pubkey-sentinel")
        bad_authority = sentinel[:13]+bytes(address)
        programdata["data"][0] = base64.b64encode(bad_authority+elf).decode()
        with self.assertRaises(AssertionError):
            verify_program_image(program, lambda key: programdata, elf)


if __name__ == "__main__":
    unittest.main()
