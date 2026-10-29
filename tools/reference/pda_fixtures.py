#!/usr/bin/env python3
"""Independent fixtures for every ABI v1 PDA (audit F-02).

Written from docs/account-layout.md, not from the Rust code: seeds are built
here byte by byte, and the address is derived with Solana's rule
(sha256(seeds || bump || program_id || "ProgramDerivedAddress"), bump from
255 down, first hash that is not an ed25519 point). The Rust test
`pda_fixtures` must reproduce every address and bump.

Usage: python3 -I tools/reference/pda_fixtures.py > docs/vectors/pda_fixtures.json
"""
import hashlib
import json
import struct

PROGRAM_ID = "B6rXDCwiXe1CyXyCS4JS1Tn9J5yud87cREWapUoD5hEy"
SCHEMA_VERSION = 1
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
P = 2**255 - 19
D = (-121665 * pow(121666, P - 2, P)) % P


def b58decode(s: str) -> bytes:
    n = 0
    for c in s:
        n = n * 58 + B58.index(c)
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big")
    return b"\0" * (len(s) - len(s.lstrip("1"))) + raw


def b58encode(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58[r] + out
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + out


def on_curve(b: bytes) -> bool:
    """True when `b` decompresses to an ed25519 point."""
    y = int.from_bytes(b, "little") & ((1 << 255) - 1)
    y2 = y * y % P
    x2 = (y2 - 1) * pow(D * y2 + 1, P - 2, P) % P
    return x2 == 0 or pow(x2, (P - 1) // 2, P) == 1


def ed25519_public(secret: bytes) -> bytes:
    """RFC 8032 public key of a 32-byte secret (affine double-and-add)."""
    h = hashlib.sha512(secret).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    by = 4 * pow(5, P - 2, P) % P
    bx2 = (by * by - 1) * pow(D * by * by + 1, P - 2, P) % P
    bx = pow(bx2, (P + 3) // 8, P)
    if (bx * bx - bx2) % P:
        bx = bx * pow(2, (P - 1) // 4, P) % P
    if bx & 1:
        bx = P - bx

    def add(p1, p2):
        (x1, y1), (x2, y2) = p1, p2
        t = D * x1 * x2 * y1 * y2 % P
        return ((x1 * y2 + x2 * y1) * pow(1 + t, P - 2, P) % P,
                (y1 * y2 + x1 * x2) * pow(1 - t, P - 2, P) % P)

    q, r = (0, 1), (bx, by)
    while a:
        if a & 1:
            q = add(q, r)
        r = add(r, r)
        a >>= 1
    return (q[1] | ((q[0] & 1) << 255)).to_bytes(32, "little")


def pda(seeds, program: bytes):
    for bump in range(255, -1, -1):
        h = hashlib.sha256(b"".join(seeds) + bytes([bump]) + program + b"ProgramDerivedAddress").digest()
        if not on_curve(h):
            return b58encode(h), bump
    raise ValueError("no bump")


def key(byte: int) -> bytes:
    return bytes([byte]) * 32


def main():
    program = b58decode(PROGRAM_ID)
    assert len(program) == 32 and b58encode(program) == PROGRAM_ID
    # RFC 8032 section 7.1, test 1.
    assert ed25519_public(bytes.fromhex(
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")).hex() == \
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    mint_a, mint_b = key(1), key(2)
    # The owner signs open_position on chain, so it is the public key of the
    # fixed secret [3; 32] (`Keypair::new_from_array([3; 32])`).
    owner = ed25519_public(key(3))
    out = []

    def add(kind, seeds, inputs):
        addr, bump = pda(seeds, program)
        out.append({"kind": kind, "inputs": inputs, "seeds_hex": [s.hex() for s in seeds],
                    "address": addr, "bump": bump})
        return b58decode(addr).rjust(32, b"\0")

    market_id = 7
    market = add("market", [b"market", struct.pack("<H", SCHEMA_VERSION), mint_a, mint_b,
                            struct.pack("<Q", market_id)],
                 {"schema": SCHEMA_VERSION, "mint_a": b58encode(mint_a),
                  "mint_b": b58encode(mint_b), "market_id": market_id})
    add("registry", [b"registry", market], {"market": b58encode(market)})
    for shard_id, generation in [(0, 0), (7, 0), (55, 3)]:
        shard = add("shard", [b"shard", market, bytes([shard_id]), struct.pack("<Q", generation)],
                    {"market": b58encode(market), "id": shard_id, "generation": generation})
        if shard_id != 0:
            continue
        add("vault_auth", [b"vault_auth", shard], {"shard": b58encode(shard)})
        for mint in (mint_a, mint_b):
            add("vault", [b"vault", shard, mint], {"shard": b58encode(shard), "mint": b58encode(mint)})
        for start in [0, -1920, 443520 - 443520 % 1920]:
            add("ticks", [b"ticks", shard, struct.pack("<i", start)],
                {"shard": b58encode(shard), "start": start})
        for lower, upper in [(-1200, 1200), (-443580, 443580), (-60, 0)]:
            add("position", [b"lp_pos", shard, owner, struct.pack("<i", lower), struct.pack("<i", upper)],
                {"shard": b58encode(shard), "owner": b58encode(owner), "lower": lower, "upper": upper})
    print(json.dumps({"program_id": PROGRAM_ID, "abi": SCHEMA_VERSION, "fixtures": out}, indent=1))


if __name__ == "__main__":
    main()
