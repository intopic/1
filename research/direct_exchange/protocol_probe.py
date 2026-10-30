"""Real Solana message/signature checks. Does NOT execute transactions in SVM.

Uses an all-zero placeholder blockhash: transactions are fixtures, not ready
for submission. TransferChecked encoding is classic SPL Token opcode 12.
"""
import hashlib
import json
import platform
import struct
import time
from pathlib import Path

from solders.hash import Hash
from solders.instruction import AccountMeta, Instruction
from solders.keypair import Keypair
from solders.pubkey import Pubkey
from solders.transaction import Transaction

from model import Candidate, exact_maximum, graph

ROOT = Path(__file__).resolve().parent
TOKEN = Pubkey.from_string("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")


def transfer(source, mint, destination, owner, amount):
    return Instruction(TOKEN, bytes([12]) + struct.pack("<Q", amount) + bytes([6]),
                       [AccountMeta(source, False, True), AccountMeta(mint, False, False),
                        AccountMeta(destination, False, True), AccountMeta(owner, True, False)])


def footprint(tx, name):
    message = tx.message
    h, keys = message.header, message.account_keys
    reads, writes = set(), set()
    for i, key in enumerate(keys):
        writable = (i < h.num_required_signatures - h.num_readonly_signed_accounts
                    if i < h.num_required_signatures else
                    i < len(keys) - h.num_readonly_unsigned_accounts)
        (writes if writable else reads).add(str(key))
    return Candidate(name, frozenset(reads), frozenset(writes))


def build_group(shared_payer=False, shared_source=False):
    mint_x, mint_y = Pubkey.new_unique(), Pubkey.new_unique()
    sponsor, common_owner, common_source = Keypair(), Keypair(), Pubkey.new_unique()
    result, instructions = [], []
    for i in range(8):
        a = common_owner if shared_source else Keypair()
        b = Keypair()
        ax = common_source if shared_source else Pubkey.new_unique()
        ay, bx, by = (Pubkey.new_unique() for _ in range(3))
        legs = [transfer(ax, mint_x, bx, a.pubkey(), 10),
                transfer(by, mint_y, ay, b.pubkey(), 20)]
        payer = sponsor if shared_payer else b
        signers = [a, b] + ([sponsor] if shared_payer else [])
        tx = Transaction.new_signed_with_payer(legs, payer.pubkey(), signers, Hash.default())
        tx.sanitize()
        tx.verify()
        result.append(tx)
        instructions.append((legs, payer, signers))
    return result, instructions


def main():
    cases = {}
    for name, kwargs in [("independent", {}), ("same_fee_payer", {"shared_payer": True}),
                         ("same_source_unique_payers", {"shared_source": True})]:
        txs, _ = build_group(**kwargs)
        adjacency = graph([footprint(tx, str(i)) for i, tx in enumerate(txs)])
        cases[name] = {"transactions": len(txs), "verified_signatures": sum(len(tx.signatures) for tx in txs),
                       "serialized_bytes": [len(bytes(tx)) for tx in txs],
                       "conflict_edges": sum(map(len, adjacency)) // 2,
                       "exact_maximum_nonconflicting": len(exact_maximum(adjacency))}
    assert cases["independent"]["exact_maximum_nonconflicting"] == 8
    assert cases["same_fee_payer"]["exact_maximum_nonconflicting"] == 1
    assert cases["same_source_unique_payers"]["exact_maximum_nonconflicting"] == 1
    txs, details = build_group()
    legs, payer, signers = details[0]
    old = txs[0]
    mutated = Instruction(legs[0].program_id, bytes([12]) + struct.pack("<Q", 11) + bytes([6]), legs[0].accounts)
    other = Transaction.new_signed_with_payer([mutated, legs[1]], payer.pubkey(), signers, Hash.default())
    tampered = Transaction.populate(other.message, old.signatures)
    try:
        tampered.verify()
    except Exception:
        tamper_rejected = True
    else:
        raise AssertionError("signature survived modified amount")
    unsigned = Transaction.new_unsigned(old.message)
    try:
        unsigned.verify()
    except Exception:
        missing_rejected = True
    else:
        raise AssertionError("missing signatures accepted")
    report = {"scope": "real Solana wire-format and Ed25519 signatures; NO SVM execution",
              "blockhash": "all-zero placeholder, not submitted",
              "token_program": str(TOKEN), "platform": platform.platform(),
              "cases": cases, "tampered_amount_rejected": tamper_rejected,
              "missing_signatures_rejected": missing_rejected,
              "source_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}
    (ROOT / "results" / "protocol_results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
