"""Execute real classic SPL SBF in LiteSVM. Serial correctness, NOT scheduler TPS.

Token/mint fixtures are injected with set_account, not initialized through CPI.
No custom order/policy/nonce contract is implemented by this experiment.
"""
import hashlib
import json
from pathlib import Path
import platform
import struct
import sys

from solders.account import Account
from solders.instruction import Instruction
from solders.keypair import Keypair
from solders.litesvm import LiteSVM
from solders.pubkey import Pubkey
from solders.transaction import Transaction
from solders.transaction_metadata import FailedTransactionMetadata

from fetch_token_elf import ELF_SHA, fetch
from protocol_probe import TOKEN, footprint, transfer
from model import exact_maximum, graph

ROOT = Path(__file__).resolve().parent


def account_image(svm, key):
    a = svm.get_account(key)
    return (a.lamports, bytes(a.data), str(a.owner), a.executable, a.rent_epoch)


def amount(svm, key):
    return struct.unpack_from("<Q", svm.get_account(key).data, 64)[0]


def account_bytes(mint, owner, balance):
    data = bytearray(165)
    data[:32], data[32:64] = bytes(mint), bytes(owner)
    struct.pack_into("<Q", data, 64, balance)
    data[108] = 1
    return bytes(data)


def setup(count=1):
    svm = LiteSVM().with_sigverify(True).with_blockhash_check(True)
    elf = fetch()
    svm.add_program(TOKEN, elf.read_bytes())
    mints = [Pubkey.new_unique(), Pubkey.new_unique()]
    for mint in mints:
        data = bytes(36) + struct.pack("<Q", count * 2000) + bytes([6, 1]) + bytes(36)
        assert len(data) == 82
        svm.set_account(mint, Account(svm.minimum_balance_for_rent_exemption(82), data, TOKEN))
    pairs, token_keys = [], []
    for _ in range(count):
        a, b = Keypair(), Keypair()
        for owner in (a, b):
            result = svm.airdrop(owner.pubkey(), 100_000_000)
            assert not isinstance(result, FailedTransactionMetadata), str(result)
        keys = [Pubkey.new_unique() for _ in range(4)]
        ax, ay, bx, by = keys
        for key, mint, owner in [(ax, mints[0], a), (ay, mints[1], a),
                                 (bx, mints[0], b), (by, mints[1], b)]:
            svm.set_account(key, Account(svm.minimum_balance_for_rent_exemption(165),
                                       account_bytes(mint, owner.pubkey(), 1000), TOKEN))
        pairs.append((a, b, ax, ay, bx, by))
        token_keys.extend(keys)
    return svm, mints, pairs, token_keys


def build(svm, mints, pair, sell_a=10, sell_b=20):
    a, b, ax, ay, bx, by = pair
    legs = [transfer(ax, mints[0], bx, a.pubkey(), sell_a),
            transfer(by, mints[1], ay, b.pubkey(), sell_b)]
    return Transaction.new_signed_with_payer(legs, b.pubkey(), [a, b], svm.latest_blockhash())


def expect_failure(svm, tx, expected):
    result = svm.send_transaction(tx)
    assert isinstance(result, FailedTransactionMetadata), f"expected failure: {result}"
    error = json.loads(result.to_json())["err"]
    assert error == expected, f"wrong failure: {error}; logs={result.meta().logs()}"
    return result


def main():
    cases = {}
    svm, mints, pairs, keys = setup(8)
    before_supply = [account_image(svm, m) for m in mints]
    initial_totals = [sum(amount(svm, key) for key in keys if svm.get_account(key).data[:32] == bytes(m)) for m in mints]
    txs = [build(svm, mints, pair) for pair in pairs]
    assert len(exact_maximum(graph([footprint(tx, str(i)) for i, tx in enumerate(txs)]))) == 8
    measurements = []
    for tx, pair in zip(txs, pairs):
        result = svm.send_transaction(tx)
        assert not isinstance(result, FailedTransactionMetadata), str(result)
        _, _, ax, ay, bx, by = pair
        assert [amount(svm, key) for key in (ax, ay, bx, by)] == [990, 1020, 1010, 980]
        assert result.compute_units_consumed() > 0
        measurements.append({"signature": str(tx.signatures[0]), "cu": result.compute_units_consumed(),
                             "logs": result.logs()})
    final_totals = [sum(amount(svm, key) for key in keys if svm.get_account(key).data[:32] == bytes(m)) for m in mints]
    assert final_totals == initial_totals == [16000, 16000]
    assert [account_image(svm, m) for m in mints] == before_supply
    cases["eight_independent_serial_runtime_executions"] = measurements

    # Leg 1 executes successfully, leg 2 fails for insufficient B funds.
    svm, mints, pairs, keys = setup()
    pair = pairs[0]
    a, b, ax, ay, bx, by = pair
    before = [account_image(svm, key) for key in keys + mints]
    payer_before = svm.get_account(b.pubkey()).lamports
    failure = expect_failure(svm, build(svm, mints, pair, sell_b=1001),
                             {"InstructionError": [1, {"Custom": 1}]})
    assert [account_image(svm, key) for key in keys + mints] == before
    assert failure.meta().logs().count(f"Program {TOKEN} success") == 1
    fee = payer_before - svm.get_account(b.pubkey()).lamports
    assert fee > 0
    cases["second_leg_failure_atomic_rollback"] = {"error": str(failure.err()), "sol_fee_lamports": fee,
                                                "logs": failure.meta().logs()}

    # Two validly signed authorizations compete for the same finite funds.
    svm, mints, pairs, keys = setup()
    pair = pairs[0]
    first = build(svm, mints, pair, sell_a=600)
    assert not isinstance(svm.send_transaction(first), FailedTransactionMetadata)
    before = [account_image(svm, key) for key in keys]
    svm.expire_blockhash()
    failure = expect_failure(svm, build(svm, mints, pair, sell_a=600),
                             {"InstructionError": [0, {"Custom": 1}]})
    assert [account_image(svm, key) for key in keys] == before
    cases["same_funds_second_spend_rejected"] = str(failure.err())

    # Identical signed bytes replay: transaction-history protection only.
    svm, mints, pairs, keys = setup()
    tx = build(svm, mints, pairs[0])
    assert not isinstance(svm.send_transaction(tx), FailedTransactionMetadata)
    before = [account_image(svm, key) for key in keys]
    expect_failure(svm, tx, "AlreadyProcessed")
    assert [account_image(svm, key) for key in keys] == before
    cases["identical_transaction_replay"] = "AlreadyProcessed"

    # Real runtime signature verification: mutate amount but reuse signatures.
    svm, mints, pairs, keys = setup()
    good = build(svm, mints, pairs[0])
    altered = build(svm, mints, pairs[0], sell_a=11)
    forged = Transaction.populate(altered.message, good.signatures)
    before = [account_image(svm, key) for key in keys]
    expect_failure(svm, forged, "SignatureFailure")
    assert [account_image(svm, key) for key in keys] == before
    cases["tampered_transaction_signature"] = "SignatureFailure"

    report = {"scope": "actual classic SPL SBF in serial LiteSVM, no custom settlement contract",
              "status": "PASS", "scheduler_speedup_measured": False,
              "platform": platform.platform(), "python": sys.version,
              "token_elf_sha256": ELF_SHA,
              "fixture_method": "preloaded valid mint/token-account bytes, not initialization transactions",
              "cases": cases,
              "source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    output = ROOT / "results" / "runtime_results.json"
    output.parent.mkdir(exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": "PASS", "cases": list(cases),
                      "cu_independent": [x["cu"] for x in measurements],
                      "scheduler_speedup_measured": False}, indent=2))


if __name__ == "__main__":
    main()
