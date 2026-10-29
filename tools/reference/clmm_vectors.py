#!/usr/bin/env python3
"""Independent exact reference for the CLMM math in programs/sharded-amm/src/math.

Written from docs/numeric-spec.md and ADR-0001 (research N05/N07), not from
the Rust code. Amounts and prices use Python integers and fractions, so every
floor/ceil is exact; tick prices use 150-digit decimal with an exact integer
fallback. Each case also checks rational invariants (rounding favors the pool,
fee accounting adds up) before it is written, and the Rust tests must
reproduce every value bit for bit.

Usage: python3 -I tools/reference/clmm_vectors.py > docs/vectors/clmm_vectors.json
"""
import json
from decimal import Decimal, ROUND_FLOOR, getcontext
from fractions import Fraction
from math import isqrt

MIN_TICK, MAX_TICK = -443636, 443636
Q64, Q128 = 1 << 64, 1 << 128
U64_MAX = (1 << 64) - 1
D = 1_000_000
FEES = [0, 100, 500, 1000, 3000, 10000, 100000]


class Rng:
    """xorshift64*, fixed so the output does not depend on Python's random."""

    def __init__(self, seed: int):
        self.x = seed

    def next(self) -> int:
        x = self.x
        x ^= (x << 13) & U64_MAX
        x ^= x >> 7
        x ^= (x << 17) & U64_MAX
        self.x = x
        return x

    def below(self, n: int) -> int:
        return self.next() % n

    def wide(self, max_bits: int) -> int:
        """Value with a random bit length, so small and huge values both occur."""
        bits = self.below(max_bits + 1)
        v = (self.next() | (self.next() << 64)) & ((1 << bits) - 1)
        return v


def floor_div(n: int, d: int) -> int:
    return n // d


def ceil_div(n: int, d: int) -> int:
    return -(-n // d)


getcontext().prec = 150
LN_BASE = Decimal("1.0001").ln()
_SQRT_CACHE = {}


def exact_sqrt_price_at_tick(t: int) -> int:
    """floor(sqrt(1.0001^t) * 2^64) with integers only. Slow for large |t|."""
    n = abs(t)
    num, den = 10001**n, 10000**n
    if t < 0:
        num, den = den, num
    return isqrt((num << 128) // den)


def sqrt_price_at_tick(t: int) -> int:
    """floor(sqrt(1.0001^t) * 2^64).

    Decimal exp/ln are correctly rounded, so at 150 digits the value is known
    to ~1e-140 relative. If it lies too close to an integer to decide the
    floor, fall back to the exact integer computation.
    """
    if t in _SQRT_CACHE:
        return _SQRT_CACHE[t]
    v = (LN_BASE * t / 2).exp() * Decimal(Q64)
    fl = int(v.to_integral_value(rounding=ROUND_FLOOR))
    frac = v - fl
    if frac < Decimal("1e-100") or frac > 1 - Decimal("1e-100"):
        fl = exact_sqrt_price_at_tick(t)
    _SQRT_CACHE[t] = fl
    return fl


MIN_SQRT = sqrt_price_at_tick(MIN_TICK)
MAX_SQRT = sqrt_price_at_tick(MAX_TICK)


def amount_a(s0: int, s1: int, liq: int, up: bool) -> int:
    sa, sb = min(s0, s1), max(s0, s1)
    exact = Fraction(liq * (sb - sa) * Q64, sa * sb)
    return ceil_div(exact.numerator, exact.denominator) if up else floor_div(exact.numerator, exact.denominator)


def amount_b(s0: int, s1: int, liq: int, up: bool) -> int:
    sa, sb = min(s0, s1), max(s0, s1)
    n = liq * (sb - sa)
    return ceil_div(n, Q64) if up else floor_div(n, Q64)


def next_from_a(s: int, liq: int, amt: int) -> int:
    if amt == 0:
        return s
    return ceil_div(liq * s * Q64, liq * Q64 + amt * s)


def next_from_b(s: int, liq: int, amt: int) -> int:
    return s + floor_div(amt * Q64, liq)


def swap_step(s: int, target: int, liq: int, gross: int, f: int):
    if s == target or gross == 0:
        return {"next": s, "input_net": 0, "fee_nominal": 0, "rounding_dust": 0, "output": 0,
                "reached": s == target}
    if liq == 0:
        return {"error": "ZeroLiquidityGap"}
    a_to_b = target < s
    budget = gross * (D - f) // D

    def needed(p0, p1):
        return amount_a(p0, p1, liq, True) if a_to_b else amount_b(p0, p1, liq, True)

    to_target = needed(s, target)
    if to_target <= budget:
        nxt, inp, reached = target, to_target, True
        fee = ceil_div(inp * f, D - f)
        dust = 0
    else:
        nxt = next_from_a(s, liq, budget) if a_to_b else next_from_b(s, liq, budget)
        inp = needed(s, nxt)
        fee = ceil_div(inp * f, D - f)
        dust = gross - inp - fee
        reached = nxt == target
    out = amount_b(nxt, s, liq, False) if a_to_b else amount_a(s, nxt, liq, False)
    if out > U64_MAX:
        return {"error": "MathOverflow"}

    # Rational invariants, independent of how the integers were derived.
    lo, hi = min(s, target), max(s, target)
    assert lo <= nxt <= hi
    if a_to_b:
        exact_in = Fraction(liq * (s - nxt) * Q64, nxt * s)
        exact_out = Fraction(liq * (s - nxt), Q64)
    else:
        exact_in = Fraction(liq * (nxt - s), Q64)
        exact_out = Fraction(liq * (nxt - s) * Q64, s * nxt)
    assert inp >= exact_in and out <= exact_out, "rounding must favor the pool"
    assert inp <= budget and fee >= Fraction(inp * f, D - f) and dust >= 0
    assert inp + fee + dust <= gross
    if not reached:
        assert inp + fee + dust == gross, "a final step consumes all input"
    return {"next": nxt, "input_net": inp, "fee_nominal": fee, "rounding_dust": dust, "output": out,
            "reached": reached}


def random_tick(rng: Rng) -> int:
    return MIN_TICK + rng.below(MAX_TICK - MIN_TICK + 1)


def random_price(rng: Rng) -> int:
    """A tick price plus a random offset toward the next tick."""
    t = random_tick(rng)
    s = sqrt_price_at_tick(t)
    if t < MAX_TICK:
        s += rng.below(sqrt_price_at_tick(t + 1) - s)
    return s


def tick_cases(rng: Rng):
    ticks = {MIN_TICK, MIN_TICK + 1, -1, 0, 1, MAX_TICK - 1, MAX_TICK}
    ticks |= {s * (1 << k) for k in range(19) for s in (1, -1)}
    ticks |= {s * m for m in (10, 60, 100000) for s in (1, -1)}
    while len(ticks) < 400:
        ticks.add(random_tick(rng))
    return [{"tick": t, "sqrt_price": str(sqrt_price_at_tick(t))} for t in sorted(ticks)]


def delta_cases(rng: Rng):
    out = []
    for _ in range(400):
        s0, s1 = random_price(rng), random_price(rng)
        liq = rng.wide(128)
        out.append({
            "s0": str(s0), "s1": str(s1), "liquidity": str(liq),
            "a_down": str(amount_a(s0, s1, liq, False)), "a_up": str(amount_a(s0, s1, liq, True)),
            "b_down": str(amount_b(s0, s1, liq, False)), "b_up": str(amount_b(s0, s1, liq, True)),
        })
    return out


def next_price_cases(rng: Rng):
    out = []
    for _ in range(300):
        s, liq, amt = random_price(rng), max(1, rng.wide(128)), rng.wide(64)
        nb = next_from_b(s, liq, amt)
        out.append({
            "s": str(s), "liquidity": str(liq), "amount": str(amt),
            "next_a": str(next_from_a(s, liq, amt)),
            "next_b": str(nb) if nb < (1 << 128) else "MathOverflow",
        })
    return out


def swap_cases(rng: Rng):
    out = []
    while len(out) < 600:
        t0 = random_tick(rng)
        # Mostly nearby targets (one interval), sometimes far ones.
        span = 20000 if rng.below(4) else (MAX_TICK - MIN_TICK)
        t1 = max(MIN_TICK, min(MAX_TICK, t0 + rng.below(2 * span + 1) - span))
        s, target = sqrt_price_at_tick(t0), sqrt_price_at_tick(t1)
        liq = 0 if rng.below(40) == 0 else rng.wide(128)
        gross = rng.wide(64)
        f = FEES[rng.below(len(FEES))]
        r = swap_step(s, target, liq, gross, f)
        row = {"s": str(s), "target": str(target), "liquidity": str(liq), "gross": str(gross), "fee_ppm": f}
        if "error" in r:
            row["error"] = r["error"]
        else:
            row.update({k: (str(v) if isinstance(v, int) and not isinstance(v, bool) else v) for k, v in r.items()})
        out.append(row)
    return out


def fee_growth_cases(rng: Rng):
    out = []
    for _ in range(200):
        lp_fee, liq = rng.wide(64), max(1, rng.wide(128))
        delta = (lp_fee << 128) // liq
        pos_liq = rng.wide(128)
        owed = pos_liq * delta // Q128
        out.append({"lp_fee": str(lp_fee), "liquidity": str(liq), "growth_delta": str(delta),
                    "position_liquidity": str(pos_liq),
                    "owed": str(owed) if owed <= U64_MAX else "MathOverflow"})
    return out


Q256 = 1 << 256


def u256(rng: Rng, max_bits: int = 256) -> int:
    bits = rng.below(max_bits + 1)
    v = 0
    for _ in range(4):
        v = (v << 64) | rng.next()
    return v & ((1 << bits) - 1)


def limb_cases(rng: Rng):
    """Borsh layout of the `[u64; 4]` fee fields: limb 0 is the least significant."""
    vals = [0, 1, U64_MAX, 1 << 64, Q128, Q128 - 1, Q256 - 1, (1 << 255) | 3]
    vals += [u256(rng) for _ in range(40)]
    out = []
    for v in vals:
        limbs = [(v >> (64 * i)) & U64_MAX for i in range(4)]
        out.append({"value": str(v), "limbs": [str(x) for x in limbs],
                    "borsh_hex": v.to_bytes(32, "little").hex()})
    return out


def fee_inside_cases(rng: Rng):
    """Random walks of one range's boundary ticks (audit F-03).

    The walk applies the protocol rules (outside on init, global - outside on
    cross, inside = global - below - above mod 2^256) and, separately, counts
    the growth that really happened while the price was inside the range.
    Every snapshot asserts the two agree for the modular difference, then
    records the inputs and the expected inside and owed values.
    """
    out = []
    lower, upper = -600, 600
    ticks = [-1200, -600, -1, 0, 599, 600, 1200]
    for walk in range(150):
        # Large starting global so wrap-around of `inside` is exercised.
        glob = u256(rng, 250) if walk % 2 else rng.wide(128) << 64
        cur = ticks[rng.below(len(ticks))]
        outside = {}

        def grow():
            nonlocal glob
            d = rng.wide(160)
            assert glob + d < Q256
            glob += d
            return d

        def move():
            nonlocal cur
            new = ticks[rng.below(len(ticks))]
            for t in outside:
                if (cur < t <= new) or (new < t <= cur):
                    outside[t] = glob - outside[t]
            cur = new

        def init(t):
            outside[t] = glob if t <= cur else 0

        # One boundary tick exists first (another position uses it); the
        # second is initialized later, which is how `inside` goes negative.
        first, second = (lower, upper) if rng.below(2) else (upper, lower)
        init(first)
        for _ in range(rng.below(5)):
            grow() if rng.below(2) else move()
        init(second)

        def inside():
            below = outside[lower] if cur >= lower else glob - outside[lower]
            above = outside[upper] if cur < upper else glob - outside[upper]
            return (glob - below - above) % Q256

        checkpoint, real = inside(), 0
        liq = max(1, rng.wide(128))
        for step in range(10):
            if rng.below(2):
                d = grow()
                if lower <= cur < upper:
                    real += d
            else:
                move()
            now = inside()
            delta = (now - checkpoint) % Q256
            assert delta == real, (walk, step)
            owed = liq * delta // Q128
            out.append({"tick_lower": lower, "tick_upper": upper, "tick_current": cur,
                        "global": str(glob), "outside_lower": str(outside[lower]),
                        "outside_upper": str(outside[upper]),
                        "inside": str(now), "checkpoint": str(checkpoint),
                        "position_liquidity": str(liq),
                        "owed": str(owed) if owed <= U64_MAX else "MathOverflow"})
    assert any(int(r["inside"]) > int(r["global"]) for r in out), "no negative inside case"
    assert any(int(r["checkpoint"]) > int(r["inside"]) for r in out), "no wrapped difference"
    return out


def main() -> None:
    rng = Rng(0x9E3779B97F4A7C15)
    # Cross-check the fast path against pure integers on a spread of ticks.
    for t in (MIN_TICK, -262144, -65536, -1, 0, 1, 4096, 131072, MAX_TICK):
        assert sqrt_price_at_tick(t) == exact_sqrt_price_at_tick(t), t
    assert MIN_SQRT == 4295048016 and MAX_SQRT == 79226673515401279992447579061
    doc = {
        "generator": "tools/reference/clmm_vectors.py",
        "min_sqrt_price_x64": str(MIN_SQRT),
        "max_sqrt_price_x64": str(MAX_SQRT),
        "ticks": tick_cases(rng),
        "amount_deltas": delta_cases(rng),
        "next_prices": next_price_cases(rng),
        "swap_steps": swap_cases(rng),
        "fee_growth": fee_growth_cases(rng),
    }
    # Separate stream so the sections above stay byte-identical.
    rng2 = Rng(0xF03F03F03F03F03)
    doc["u256_limbs"] = limb_cases(rng2)
    doc["fee_inside"] = fee_inside_cases(rng2)
    print(json.dumps(doc, indent=1))


if __name__ == "__main__":
    main()
