//! What a client does when a planned swap fails.
//!
//! The program's errors already tell apart "the client's snapshot is stale or
//! incomplete" from "this shard has no liquidity there" from "the market is
//! stopped". [`classify`] turns an error into one [`Recovery`], and
//! [`replan`] plans again without the shards that cannot fill the order, so a
//! client never retries a request that will fail the same way. [`after_send`]
//! handles a send whose outcome is unknown (an RPC timeout): same bytes
//! while the blockhash is valid, a new order only after expiry and a full
//! history search.

use crate::{plan, Book, MarketCfg, Plan, RouterConfig};
use sharded_amm::constants::MAX_SWAP_TICK_ARRAYS;
use sharded_amm::errors::AmmError;

/// Anchor's offset for program error codes.
pub const ERROR_CODE_OFFSET: u32 = 6000;

/// Every program error in code order (codes are append-only).
const ALL: [AmmError; 67] = [
    AmmError::ShardIdOutOfRange,
    AmmError::ReservedBitsSet,
    AmmError::ActiveNotPrepared,
    AmmError::ShardAlreadyPrepared,
    AmmError::ShardNotPrepared,
    AmmError::ShardAlreadyActive,
    AmmError::ShardNotActive,
    AmmError::EmptyMask,
    AmmError::RankOutOfRange,
    AmmError::RegistryVersionOverflow,
    AmmError::InvalidMarketStatus,
    AmmError::NotAnInitialShard,
    AmmError::InitialShardsNotPrepared,
    AmmError::InitialShardsNotReady,
    AmmError::ShardNotReady,
    AmmError::InvalidFeeRate,
    AmmError::InvalidProtocolShare,
    AmmError::InvalidTickSpacing,
    AmmError::MintsNotCanonical,
    AmmError::Unauthorized,
    AmmError::UnsupportedGeneration,
    AmmError::MathOverflow,
    AmmError::DivisionByZero,
    AmmError::TickOutOfRange,
    AmmError::SqrtPriceOutOfRange,
    AmmError::ZeroLiquidityGap,
    AmmError::ZeroLiquidity,
    AmmError::MathInvariantViolated,
    AmmError::PriceNotInitialized,
    AmmError::PriceAlreadyInitialized,
    AmmError::InvalidTickRange,
    AmmError::InvalidTickArrayStart,
    AmmError::TickArrayMismatch,
    AmmError::TickArraysExhausted,
    AmmError::InvalidPriceLimit,
    AmmError::PriceLimitReached,
    AmmError::TooManySwapSteps,
    AmmError::ZeroAmount,
    AmmError::SlippageExceeded,
    AmmError::RoundingDustExceeded,
    AmmError::DeadlineExceeded,
    AmmError::ZeroOutput,
    AmmError::LiquidityPerTickExceeded,
    AmmError::PositionNotEmpty,
    AmmError::InsufficientPositionLiquidity,
    AmmError::InvalidVault,
    AmmError::InvalidTokenAccount,
    AmmError::DuplicateAccount,
    AmmError::ShardMismatch,
    AmmError::PriceDeviationTooHigh,
    AmmError::TimelockNotElapsed,
    AmmError::NoPendingChange,
    AmmError::InvalidReadinessParams,
    AmmError::InvalidAccountCount,
    AmmError::InvalidDynamicFeeParams,
    AmmError::SwapsPaused,
    AmmError::DepositsPaused,
    AmmError::RebalancePaused,
    AmmError::InvalidPauseFlags,
    AmmError::RebalanceUnprofitable,
    AmmError::RebalanceNoPriceGap,
    AmmError::NotRebalancer,
    AmmError::InvalidRebalancers,
    AmmError::InsufficientDepth,
    AmmError::ShardPaused,
    AmmError::TransferAmountMismatch,
    AmmError::ReopenNeedsPriceCheck,
];

/// The program error behind a custom instruction error code.
pub fn decode(code: u32) -> Option<AmmError> {
    let i = code.checked_sub(ERROR_CODE_OFFSET)?;
    ALL.get(i as usize).copied()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// The snapshot was stale or missing tick arrays: fetch the shard and its
    /// arrays again, then plan again.
    Refresh,
    /// This shard cannot fill the order now (no liquidity in the way, not
    /// active, not priced, paused on its own): plan again without it.
    SkipShard,
    /// One leg needs more steps or arrays than a transaction can carry: plan
    /// again with the order spread over more shards, or a smaller amount.
    Smaller,
    /// The market does not take swaps now: do not retry until its state
    /// changes.
    Halt,
    /// The request itself is wrong (amount, limit, accounts): the same request
    /// fails the same way.
    Reject,
}

/// How to recover from `e` on a leg that passed `arrays_passed` tick arrays.
pub fn classify(e: AmmError, arrays_passed: usize) -> Recovery {
    use AmmError::*;
    match e {
        TickArraysExhausted if arrays_passed >= MAX_SWAP_TICK_ARRAYS => Recovery::Smaller,
        SlippageExceeded | PriceLimitReached | TickArraysExhausted | TickArrayMismatch
        | DeadlineExceeded | RoundingDustExceeded => Recovery::Refresh,
        ZeroLiquidityGap | ShardNotActive | PriceNotInitialized | ShardNotReady | ShardPaused => {
            Recovery::SkipShard
        }
        TooManySwapSteps => Recovery::Smaller,
        SwapsPaused | InvalidMarketStatus | UnsupportedGeneration => Recovery::Halt,
        _ => Recovery::Reject,
    }
}

/// What the client should do next after leg `failed` of a plan failed with `e`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Send this plan instead (same books).
    Send(Plan),
    /// Fetch fresh books, then call [`plan`] again.
    Refresh,
    /// Stop; retrying cannot help.
    Stop(Recovery),
}

/// The next step after a failure, planned on the same `books` when that is
/// enough: without the failed shard for [`Recovery::SkipShard`], with no
/// split threshold (as many shards as still help) for [`Recovery::Smaller`].
#[allow(clippy::too_many_arguments)]
pub fn replan(
    books: &[Book],
    m: &MarketCfg,
    slot: u64,
    a_to_b: bool,
    amount_in: u64,
    cfg: RouterConfig,
    seed: u64,
    failed: u8,
    e: AmmError,
    arrays_passed: usize,
) -> Next {
    let r = classify(e, arrays_passed);
    let p = match r {
        Recovery::Refresh => return Next::Refresh,
        Recovery::Halt | Recovery::Reject => return Next::Stop(r),
        Recovery::SkipShard => {
            let rest: Vec<Book> = books.iter().filter(|b| b.id != failed).cloned().collect();
            plan(&rest, m, slot, a_to_b, amount_in, cfg, seed)
        }
        Recovery::Smaller => {
            let wide = RouterConfig {
                min_gain_bps: 0,
                split_tax_bps: 0,
                leg_cost_out: 0,
                ..cfg
            };
            plan(books, m, slot, a_to_b, amount_in, wide, seed)
        }
    };
    p.map_or(Next::Stop(r), Next::Send)
}

/// What the client found when it asked the RPC for a sent transaction's
/// status by signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxStatus {
    /// Executed and succeeded.
    Landed,
    /// Executed and failed with this custom program error code.
    Failed(u32),
    /// Executed and failed with an error that is not the program's.
    FailedOther,
    /// Not seen. `history`: the lookup searched the full transaction history
    /// (`searchTransactionHistory`), not only the RPC node's recent cache.
    NotFound { history: bool },
}

/// The next step after a send whose outcome is unknown, e.g. an RPC timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterSend {
    /// It went through: never send the order again.
    Done,
    /// Not seen and its blockhash still valid: send the *same* signed bytes
    /// again (a duplicate is dropped by the cluster) and check again.
    ResendSame,
    /// Not seen in the full history and its blockhash expired, so it can no
    /// longer be accepted: a new quote and a new transaction may be sent.
    Expired,
    /// The blockhash expired but only the recent cache was searched: a
    /// `null` there does not prove it never ran. Search the history first.
    CheckHistory,
    /// It failed: recover as the error says.
    Recover(Recovery),
}

/// Decides what to do with a sent transaction so that a new one is sent
/// only once the old one failed, or its blockhash can no longer be used
/// (`block_height > last_valid_block_height`) *and* the full history shows
/// it did not run. This narrows the window for a duplicate trade; it does
/// not make one impossible (e.g. an RPC that answers wrongly), which only an
/// order id consumed onchain would.
pub fn after_send(
    status: TxStatus,
    block_height: u64,
    last_valid_block_height: u64,
    arrays_passed: usize,
) -> AfterSend {
    match status {
        TxStatus::Landed => AfterSend::Done,
        TxStatus::Failed(code) => AfterSend::Recover(
            decode(code).map_or(Recovery::Reject, |e| classify(e, arrays_passed)),
        ),
        TxStatus::FailedOther => AfterSend::Recover(Recovery::Reject),
        TxStatus::NotFound { .. } if block_height <= last_valid_block_height => {
            AfterSend::ResendSame
        }
        TxStatus::NotFound { history: false } => AfterSend::CheckHistory,
        TxStatus::NotFound { history: true } => AfterSend::Expired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_decode_to_every_error_in_order() {
        let src = include_str!("../../../programs/sharded-amm/src/errors.rs");
        let body = &src[src.find("pub enum AmmError").unwrap()..];
        let body = &body[..body.find("\n}").unwrap()];
        let variants = body
            .lines()
            .filter(|l| {
                let t = l.trim();
                t.ends_with(',') && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            })
            .count();
        assert_eq!(variants, ALL.len(), "update ALL when an error is added");
        for (i, e) in ALL.iter().enumerate() {
            assert_eq!(u32::from(*e), ERROR_CODE_OFFSET + i as u32, "{e:?}");
            assert_eq!(decode(u32::from(*e)), Some(*e));
        }
        assert_eq!(decode(ERROR_CODE_OFFSET + ALL.len() as u32), None);
        assert_eq!(decode(1), None);
    }

    #[test]
    fn a_timeout_is_resolved_before_any_new_order() {
        use TxStatus::*;
        let (recent, full) = (NotFound { history: false }, NotFound { history: true });
        // Unknown and still valid: the same bytes again, never a new order.
        assert_eq!(after_send(recent, 100, 150, 1), AfterSend::ResendSame);
        assert_eq!(after_send(full, 150, 150, 1), AfterSend::ResendSame);
        // Expired: a recent-cache miss is not proof; the history is.
        assert_eq!(after_send(recent, 151, 150, 1), AfterSend::CheckHistory);
        assert_eq!(after_send(full, 151, 150, 1), AfterSend::Expired);
        assert_eq!(after_send(Landed, 151, 150, 1), AfterSend::Done);
        let slip = u32::from(AmmError::SlippageExceeded);
        assert_eq!(
            after_send(Failed(slip), 120, 150, 1),
            AfterSend::Recover(Recovery::Refresh)
        );
        assert_eq!(
            after_send(Failed(u32::from(AmmError::ShardPaused)), 120, 150, 1),
            AfterSend::Recover(Recovery::SkipShard)
        );
        // Unknown codes and non-program errors are not retried blindly.
        assert_eq!(
            after_send(Failed(1), 120, 150, 1),
            AfterSend::Recover(Recovery::Reject)
        );
        assert_eq!(
            after_send(FailedOther, 120, 150, 1),
            AfterSend::Recover(Recovery::Reject)
        );
    }

    #[test]
    fn missing_data_is_not_missing_liquidity() {
        assert_eq!(
            classify(AmmError::TickArraysExhausted, 1),
            Recovery::Refresh
        );
        assert_eq!(
            classify(AmmError::TickArraysExhausted, MAX_SWAP_TICK_ARRAYS),
            Recovery::Smaller
        );
        assert_eq!(classify(AmmError::ZeroLiquidityGap, 1), Recovery::SkipShard);
        assert_eq!(classify(AmmError::SlippageExceeded, 1), Recovery::Refresh);
        assert_eq!(classify(AmmError::SwapsPaused, 1), Recovery::Halt);
        assert_eq!(classify(AmmError::ShardPaused, 1), Recovery::SkipShard);
        assert_eq!(classify(AmmError::ZeroAmount, 1), Recovery::Reject);
        assert_eq!(classify(AmmError::InvalidVault, 1), Recovery::Reject);
    }

    #[test]
    fn replan_skips_the_failed_shard_and_stops_when_it_cannot_help() {
        let (bs, m) = (crate::tests::books(8, 1 << 40), crate::tests::cfg());
        let rc = RouterConfig::default();
        let p = plan(&bs, &m, 0, true, 1 << 20, rc, 7).unwrap();
        assert_eq!(p.legs.len(), 1);
        let failed = p.legs[0].id;
        let Next::Send(q) = replan(
            &bs,
            &m,
            0,
            true,
            1 << 20,
            rc,
            7,
            failed,
            AmmError::ZeroLiquidityGap,
            1,
        ) else {
            panic!("expected a new plan")
        };
        assert!(q.legs.iter().all(|l| l.id != failed));
        let next = |e| replan(&bs, &m, 0, true, 1 << 20, rc, 7, failed, e, 1);
        assert_eq!(next(AmmError::SlippageExceeded), Next::Refresh);
        assert_eq!(next(AmmError::SwapsPaused), Next::Stop(Recovery::Halt));
        assert_eq!(next(AmmError::ZeroAmount), Next::Stop(Recovery::Reject));
        // Only one shard and it has no liquidity: nothing left to try.
        assert_eq!(
            replan(
                &bs[..1],
                &m,
                0,
                true,
                1 << 20,
                rc,
                7,
                bs[0].id,
                AmmError::ZeroLiquidityGap,
                1
            ),
            Next::Stop(Recovery::SkipShard)
        );
    }

    #[test]
    fn too_many_steps_spreads_the_order_over_more_shards() {
        let (bs, m) = (crate::tests::books(8, 1 << 40), crate::tests::cfg());
        let rc = RouterConfig::default();
        let amount = 1 << 33;
        let p = plan(&bs, &m, 0, true, amount, rc, 7).unwrap();
        let Next::Send(q) = replan(
            &bs,
            &m,
            0,
            true,
            amount,
            rc,
            7,
            p.legs[0].id,
            AmmError::TooManySwapSteps,
            1,
        ) else {
            panic!("expected a wider plan")
        };
        assert!(
            q.legs.len() > p.legs.len(),
            "{} -> {}",
            p.legs.len(),
            q.legs.len()
        );
        assert!(q.legs.iter().all(|l| l.amount_in < p.legs[0].amount_in));
    }
}
