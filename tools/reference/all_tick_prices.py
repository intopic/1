#!/usr/bin/env python3
"""Writes floor(sqrt(1.0001^t) * 2^64) for every tick in [MIN_TICK, MAX_TICK]
as little-endian u128 values to the given file (about 14 MB).

Each value comes from a 80-digit Decimal power walked outward from tick 0.
A value whose fractional part lies within 1e-30 of an integer would be
ambiguous; the script recomputes those exactly with integer square roots.
Used by the stress workflow: the Rust test `full_tick_table` compares every
tick against this file when AMM_FULL_TICK_TABLE points to it.
"""
import sys
from decimal import Decimal, getcontext
from math import isqrt

MAX_TICK = 443636
getcontext().prec = 80
TWO64 = Decimal(2) ** 64
EPS = Decimal(10) ** -30


def exact(t: int) -> int:
    if t >= 0:
        return isqrt((10001**t << 128) // 10000**t)
    return isqrt((10000 ** (-t) << 128) // 10001 ** (-t))


def value(x: Decimal, t: int) -> int:
    y = x * TWO64
    f = int(y)
    frac = y - f
    if frac < EPS or 1 - frac < EPS:
        return exact(t)
    return f


def main() -> None:
    root_up = Decimal("1.0001").sqrt()
    root_down = 1 / root_up
    vals = {0: 1 << 64}
    x = Decimal(1)
    for t in range(1, MAX_TICK + 1):
        x *= root_up
        vals[t] = value(x, t)
    x = Decimal(1)
    for t in range(1, MAX_TICK + 1):
        x *= root_down
        vals[-t] = value(x, -t)
    # Spot-check the walk against exact roots at a few ticks.
    for t in (1, -1, 777, -777, 100_000, -100_000, MAX_TICK, -MAX_TICK):
        assert vals[t] == exact(t), t
    with open(sys.argv[1], "wb") as f:
        for t in range(-MAX_TICK, MAX_TICK + 1):
            f.write(vals[t].to_bytes(16, "little"))


if __name__ == "__main__":
    main()
