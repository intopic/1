"""Research model only: lock conflicts and one CLMM band; no chain benchmark."""
from fractions import Fraction
import json
from pathlib import Path


def conflict(left, right):
    lr, lw = left
    rr, rw = right
    return bool(lw & (rr | rw) or rw & lr)


def swap_accounts(shard, user, *, shared_vault=False, shared_payer=False):
    reads = {"market_config", "token_program"}
    writes = {f"shard:{shard}", f"ticks:{shard}",
              f"vault_x:{'global' if shared_vault else shard}",
              f"vault_y:{'global' if shared_vault else shard}",
              f"user_x:{user}", f"user_y:{user}",
              f"payer:{'global' if shared_payer else user}"}
    return reads, writes


def output_token1(liquidity, sqrt_price, input_token0):
    # Exact input, zero fees, no tick crossing; virtual CLMM reserves.
    new_sqrt = liquidity * sqrt_price / (liquidity + input_token0 * sqrt_price)
    return liquidity * (sqrt_price - new_sqrt)


def main():
    a = swap_accounts(0, "alice")
    cases = {
        "independent_shards": conflict(a, swap_accounts(1, "bob")),
        "same_shard": conflict(a, swap_accounts(0, "bob")),
        "shared_vaults": conflict(swap_accounts(0, "alice", shared_vault=True),
                                 swap_accounts(1, "bob", shared_vault=True)),
        "shared_fee_payer": conflict(swap_accounts(0, "alice", shared_payer=True),
                                    swap_accounts(1, "bob", shared_payer=True)),
        "same_user_token_accounts": conflict(a, swap_accounts(1, "alice")),
    }
    assert cases == {"independent_shards": False, "same_shard": True,
                     "shared_vaults": True, "shared_fee_payer": True,
                     "same_user_token_accounts": True}
    full = output_token1(Fraction(1000), Fraction(1), Fraction(100))
    one_shard = output_token1(Fraction(500), Fraction(1), Fraction(100))
    both = 2 * output_token1(Fraction(500), Fraction(1), Fraction(50))
    assert both == full
    assert one_shard < full
    assert (full - one_shard) / full == Fraction(1, 12)
    report = {
        "scope": "Symbolic account-set and rational-arithmetic model; no SVM, network, TPS, CU or latency measurement.",
        "lock_conflict_present": cases,
        "no_fee_single_band_example": {
            "initial_sqrt_price": 1, "total_liquidity": 1000,
            "token0_input": 100, "unsharded_token1_output": float(full),
            "one_of_two_shards_token1_output": float(one_shard),
            "proportional_two_shard_output": float(both),
            "single_shard_output_loss_percent": float((full-one_shard)/full*100),
        },
        "model_checks_passed": 8,
    }
    output = Path(__file__).with_name("probe_results.json")
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
