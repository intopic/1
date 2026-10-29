#!/usr/bin/env python3
"""R-01: independent reference for a whole exact-in swap (docs/numeric-spec.md
sections 2, 5 and 6), written from the specification and not from the Rust
code.

The per-step math (amounts, next price, fee of a step) comes from
clmm_vectors.py, itself derived from the spec with exact integers. This file
adds what a whole swap does on top: the price limit, walking the declared
tick arrays, crossing initialized ticks (liquidity and fee growth outside),
the fee split and fee growth per step, the step cap and the final checks.

Input: one JSON object per line on stdin (a shard snapshot and a swap, as
written by tests/svm/tests/r01_swap_reference.rs). Output: one JSON object per
line, either {"ok": {...}} with the state after the swap, or {"err": name}.
Integers that may exceed 2^53 are decimal strings both ways.

Usage: python3 -I tools/reference/swap_reference.py < cases.jsonl
"""
import json
import os
import sys
from decimal import Decimal

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from clmm_vectors import (  # noqa: E402  (the path is set just above)
    D, LN_BASE, MAX_SQRT, MAX_TICK, MIN_SQRT, MIN_TICK, Q128, U64_MAX,
    sqrt_price_at_tick, swap_step,
)

TICKS_PER_ARRAY = 32
MAX_SWAP_STEPS = 128
U256 = 1 << 256


class SwapError(Exception):
    pass


def tick_at_sqrt_price(s: int) -> int:
    """Greatest t with sqrt_price_at_tick(t) <= s (section 2)."""
    if not MIN_SQRT <= s <= MAX_SQRT:
        raise SwapError("InvalidSqrtPrice")
    est = int(((Decimal(s) / Decimal(1 << 64)).ln() * 2 / LN_BASE).to_integral_value())
    t = max(MIN_TICK, min(MAX_TICK, est))
    while t > MIN_TICK and sqrt_price_at_tick(t) > s:
        t -= 1
    while t < MAX_TICK and sqrt_price_at_tick(t + 1) <= s:
        t += 1
    return t


def swap(c: dict) -> dict:
    a_to_b = c["a_to_b"]
    f = c["fee_rate_ppm"]
    share = c["protocol_share_bps"]
    spacing = c["tick_spacing"]
    span = TICKS_PER_ARRAY * spacing
    s = int(c["sqrt_price"])
    tick = c["tick"]
    liq = int(c["liquidity"])
    growth = [int(c["growth_a"]), int(c["growth_b"])]
    protocol = [int(c["protocol_a"]), int(c["protocol_b"])]
    remaining = int(c["amount_in"])
    if remaining == 0:
        raise SwapError("ZeroAmount")

    # Price limit (section 5.1); 0 means the domain bound. Limits are
    # strictly inside the domain so the tick stays in range.
    limit = int(c["limit"])
    if limit == 0:
        limit = MIN_SQRT + 1 if a_to_b else MAX_SQRT - 1
    if a_to_b and not MIN_SQRT < limit < s:
        raise SwapError("InvalidPriceLimit")
    if not a_to_b and not s < limit < MAX_SQRT:
        raise SwapError("InvalidPriceLimit")

    # Declared arrays (section 5.2): the first holds the current tick, the
    # others follow it in the swap direction.
    starts = c["array_starts"]
    if len(starts) > 3:
        raise SwapError("InvalidAccountCount")
    if not starts or starts[0] != (tick // span) * span:
        raise SwapError("TickArrayMismatch")
    for prev, nxt_start in zip(starts, starts[1:]):
        if nxt_start != prev + (-span if a_to_b else span):
            raise SwapError("TickArrayMismatch")
    ticks = {int(t["index"]): {"net": int(t["net"]), "out_a": int(t["out_a"]), "out_b": int(t["out_b"])}
             for t in c["ticks"]}
    crossed = {}

    total_in = out_total = fee_total = prot_total = dust_total = 0
    steps = 0
    ai = 0
    while remaining > 0:
        steps += 1
        if steps > MAX_SWAP_STEPS:
            raise SwapError("TooManySwapSteps")
        if ai >= len(starts):
            raise SwapError("TickArraysExhausted")
        # One array at a time: the next initialized tick in this array (at
        # or below the current tick going down, above it going up), or the
        # array's edge, which is a step boundary of its own.
        start = starts[ai]
        lo, hi = start, start + span
        if a_to_b:
            cand = [t for t in ticks if lo <= t <= tick and t < hi]
            found = max(cand) if cand else None
            target_tick = found if found is not None else max(start, MIN_TICK)
        else:
            cand = [t for t in ticks if max(tick + 1, lo) <= t < hi]
            found = min(cand) if cand else None
            target_tick = found if found is not None else min(start + span, MAX_TICK)
        tick_price = sqrt_price_at_tick(target_tick)
        target = max(tick_price, limit) if a_to_b else min(tick_price, limit)
        r = swap_step(s, target, liq, remaining, f)
        if "error" in r:
            raise SwapError(r["error"])
        inp, fee, dust, out = r["input_net"], r["fee_nominal"], r["rounding_dust"], r["output"]
        step_fee = fee + dust
        k = 0 if a_to_b else 1
        if step_fee > 0:
            prot = step_fee * share // 10_000
            lp = step_fee - prot
            if lp > 0:
                # Fees accrue with the liquidity before crossing (section 5).
                growth[k] += (lp << 128) // liq
                if growth[k] >= U256:
                    raise SwapError("MathOverflow")
            protocol[k] += prot
            if protocol[k] > U64_MAX:
                raise SwapError("MathOverflow")
            prot_total += prot
        remaining -= inp + step_fee
        total_in += inp + step_fee
        out_total += out
        if out_total > U64_MAX:
            raise SwapError("MathOverflow")
        fee_total += step_fee
        dust_total += dust
        moved = r["next"] != s
        s = r["next"]
        if r["reached"] and target == tick_price:
            if found is not None:
                t = ticks[found]
                liq = liq - t["net"] if a_to_b else liq + t["net"]
                if liq < 0 or liq >= 1 << 128:
                    raise SwapError("MathOverflow")
                # The new outside is taken with the growth at crossing time.
                crossed[found] = (growth[0], growth[1])
                tick = found - 1 if a_to_b else found
                if not lo <= tick < hi:
                    ai += 1
            else:
                # Array edge: the next search starts in the next array; the
                # edge tick stays eligible there (moving up it is that
                # array's first tick).
                tick = target_tick - 1
                ai += 1
        else:
            # Stopped inside an interval or at the limit. A zero step right
            # after a downward crossing keeps tick = crossed - 1.
            if moved:
                tick = tick_at_sqrt_price(s)
            if r["reached"] and remaining > 0:
                raise SwapError("PriceLimitReached")
    if out_total == 0:
        raise SwapError("ZeroOutput")
    for i, (ga, gb) in crossed.items():
        t = ticks[i]
        if t["out_a"] > ga or t["out_b"] > gb:
            raise SwapError("MathInvariantViolated")
        t["out_a"], t["out_b"] = ga - t["out_a"], gb - t["out_b"]
    if out_total < int(c["min_amount_out"]):
        raise SwapError("SlippageExceeded")
    if dust_total > int(c["max_rounding_dust"]):
        raise SwapError("RoundingDustExceeded")
    return {
        "amount_in": str(total_in),
        "amount_out": str(out_total),
        "total_fee": str(fee_total),
        "protocol_fee": str(prot_total),
        "rounding_dust": str(dust_total),
        "sqrt_price": str(s),
        "tick": tick,
        "liquidity": str(liq),
        "growth_a": str(growth[0]),
        "growth_b": str(growth[1]),
        "protocol_a": str(protocol[0]),
        "protocol_b": str(protocol[1]),
        "crossed": [{"index": i, "out_a": str(ticks[i]["out_a"]), "out_b": str(ticks[i]["out_b"])}
                    for i in sorted(crossed)],
    }


def main() -> None:
    for line in sys.stdin:
        if not line.strip():
            continue
        c = json.loads(line)
        try:
            res = {"ok": swap(c)}
        except SwapError as e:
            res = {"err": str(e)}
        print(json.dumps(res), flush=True)


if __name__ == "__main__":
    main()
