#!/usr/bin/env python3
"""Independent high-precision reference for the Q64.64 tick/sqrt-price domain.

Uses only the Python standard library (decimal, 120 significant digits). It
computes sqrt(1.0001^tick) * 2^64 for selected ticks and emits exact floor and
ceil integers, so the on-chain tick_math (ENG-05) can be differentially tested
against values that were not produced by the code under test.

Usage: python3 -I tools/reference/tick_vectors.py > docs/vectors/tick_vectors.json
"""
import json
from decimal import Decimal, getcontext, ROUND_FLOOR, ROUND_CEILING

getcontext().prec = 120
Q64 = Decimal(2) ** 64
BASE = Decimal("1.0001")
MIN_TICK, MAX_TICK = -443636, 443636


def sqrt_price_x64(tick: int) -> Decimal:
    # 1.0001^(tick/2) computed as exp(ln(1.0001) * tick / 2).
    return (BASE.ln() * Decimal(tick) / 2).exp() * Q64


def as_int(d: Decimal, rounding) -> int:
    return int(d.to_integral_value(rounding=rounding))


def main() -> None:
    ticks = sorted({MIN_TICK, MIN_TICK + 1, -100000, -60, -1, 0, 1, 60, 100000, MAX_TICK - 1, MAX_TICK}
                   | {s * 2**k for k in range(0, 19) for s in (1, -1) if 2**k <= MAX_TICK})
    rows = []
    for t in ticks:
        v = sqrt_price_x64(t)
        rows.append({"tick": t, "floor": str(as_int(v, ROUND_FLOOR)), "ceil": str(as_int(v, ROUND_CEILING))})
    lo = sqrt_price_x64(MIN_TICK)
    hi = sqrt_price_x64(MAX_TICK)
    # The domain must keep sqrt prices inside u128 with headroom and above zero.
    assert lo > 1 and hi < Decimal(2) ** 96
    out = {
        "generator": "tools/reference/tick_vectors.py",
        "precision_digits": getcontext().prec,
        "definition": "sqrt_price_x64(tick) = sqrt(1.0001^tick) * 2^64",
        "min_tick": MIN_TICK,
        "max_tick": MAX_TICK,
        "min_sqrt_price_x64_floor": str(as_int(lo, ROUND_FLOOR)),
        "max_sqrt_price_x64_ceil": str(as_int(hi, ROUND_CEILING)),
        "max_sqrt_price_bits": as_int(hi, ROUND_CEILING).bit_length(),
        "vectors": rows,
    }
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
