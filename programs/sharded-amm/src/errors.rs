use anchor_lang::prelude::*;

/// Program error codes. Codes are part of the ABI: append only, never reorder.
#[error_code]
#[derive(PartialEq, Eq)]
pub enum AmmError {
    #[msg("Shard ID must be below MAX_SHARDS (56)")]
    ShardIdOutOfRange,
    #[msg("Mask has reserved bits 56..=63 set")]
    ReservedBitsSet,
    #[msg("Active mask is not a subset of the prepared mask")]
    ActiveNotPrepared,
    #[msg("Shard is already prepared")]
    ShardAlreadyPrepared,
    #[msg("Shard is not prepared")]
    ShardNotPrepared,
    #[msg("Shard is already active")]
    ShardAlreadyActive,
    #[msg("Shard is not active")]
    ShardNotActive,
    #[msg("Mask is empty")]
    EmptyMask,
    #[msg("Rank is not below the number of set bits")]
    RankOutOfRange,
    #[msg("Registry version overflow")]
    RegistryVersionOverflow,
    #[msg("Operation not allowed in the current market status")]
    InvalidMarketStatus,
    #[msg("Only IDs below INITIAL_SHARDS can be prepared before the market opens")]
    NotAnInitialShard,
    #[msg("All initial shards must be prepared before finalization")]
    InitialShardsNotPrepared,
    #[msg("All initial shards must pass readiness checks in the finalizing transaction")]
    InitialShardsNotReady,
    #[msg("Shard failed readiness checks")]
    ShardNotReady,
    #[msg("Fee rate above MAX_FEE_RATE_PPM")]
    InvalidFeeRate,
    #[msg("Protocol share above MAX_PROTOCOL_SHARE_BPS")]
    InvalidProtocolShare,
    #[msg("Tick spacing outside 1..=MAX_TICK_SPACING")]
    InvalidTickSpacing,
    #[msg("Mints must be distinct and in canonical order (mint_a < mint_b)")]
    MintsNotCanonical,
    #[msg("Signer does not hold the required role")]
    Unauthorized,
    #[msg("Only generation 0 is supported in v1")]
    UnsupportedGeneration,
    #[msg("Arithmetic overflow or result does not fit the target type")]
    MathOverflow,
    #[msg("Division by zero")]
    DivisionByZero,
    #[msg("Tick outside MIN_TICK..=MAX_TICK")]
    TickOutOfRange,
    #[msg("Sqrt price outside MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64")]
    SqrtPriceOutOfRange,
    #[msg("Swap reached a price interval with zero liquidity")]
    ZeroLiquidityGap,
    #[msg("Liquidity must be positive")]
    ZeroLiquidity,
    #[msg("Internal math invariant violated")]
    MathInvariantViolated,
    #[msg("Shard price is not initialized")]
    PriceNotInitialized,
    #[msg("Shard price is already initialized")]
    PriceAlreadyInitialized,
    #[msg("Ticks must be multiples of tick_spacing with MIN_TICK <= lower < upper <= MAX_TICK")]
    InvalidTickRange,
    #[msg("Tick array start index is not aligned or outside the domain")]
    InvalidTickArrayStart,
    #[msg("Tick array missing, out of order, or belongs to another shard")]
    TickArrayMismatch,
    #[msg("Swap needs more tick arrays than were provided")]
    TickArraysExhausted,
    #[msg("Price limit is on the wrong side of the current price or outside the domain")]
    InvalidPriceLimit,
    #[msg("Price limit reached before the input was consumed; partial fills are not supported")]
    PriceLimitReached,
    #[msg("Swap exceeded MAX_SWAP_STEPS")]
    TooManySwapSteps,
    #[msg("Amount must be positive")]
    ZeroAmount,
    #[msg("Output below the minimum or input above the maximum")]
    SlippageExceeded,
    #[msg("Rounding dust above the caller's max_rounding_dust")]
    RoundingDustExceeded,
    #[msg("Transaction deadline has passed")]
    DeadlineExceeded,
    #[msg("Swap output is zero")]
    ZeroOutput,
    #[msg("Liquidity per tick above the cap for this tick spacing")]
    LiquidityPerTickExceeded,
    #[msg("Position still holds liquidity or uncollected tokens")]
    PositionNotEmpty,
    #[msg("Position liquidity is lower than the requested decrease")]
    InsufficientPositionLiquidity,
    #[msg("Vault does not belong to this shard")]
    InvalidVault,
    #[msg("Token account has the wrong mint or owner")]
    InvalidTokenAccount,
    #[msg("The same account was passed twice")]
    DuplicateAccount,
    #[msg("Account belongs to another market or shard")]
    ShardMismatch,
    #[msg("Shard price deviates from the reference shard by more than allowed")]
    PriceDeviationTooHigh,
    #[msg("Fee change timelock has not elapsed")]
    TimelockNotElapsed,
    #[msg("No pending fee change")]
    NoPendingChange,
    #[msg("Invalid readiness parameters")]
    InvalidReadinessParams,
    #[msg("Wrong number of accounts for this instruction")]
    InvalidAccountCount,
    #[msg("Invalid dynamic fee parameters")]
    InvalidDynamicFeeParams,
    #[msg("Swaps are paused")]
    SwapsPaused,
    #[msg("New liquidity is paused")]
    DepositsPaused,
    #[msg("Rebalancing is paused")]
    RebalancePaused,
    #[msg("Unknown pause flag bits")]
    InvalidPauseFlags,
    #[msg("Rebalance would not pay for itself")]
    RebalanceUnprofitable,
    #[msg("Rebalance needs two shards with different prices")]
    RebalanceNoPriceGap,
    #[msg("Signer is not one of the market's rebalancers")]
    NotRebalancer,
    #[msg("Rebalancer list has a duplicate key")]
    InvalidRebalancers,
    #[msg("Probe trade moved the price more than the readiness limit")]
    InsufficientDepth,
    #[msg("This operation is paused on this shard")]
    ShardPaused,
    #[msg("A token transfer did not move exactly the requested amount")]
    TransferAmountMismatch,
    #[msg("Lifting a shard's swap pause needs reopen_shard and its price check")]
    ReopenNeedsPriceCheck,
    #[msg("Invalid protocol settings")]
    InvalidProtocolSettings,
    #[msg("Market profile is not allowed by the protocol")]
    ProfileNotAllowed,
    #[msg("Market roles are out of date; call sync_market_roles")]
    StaleRoles,
    #[msg("Mint has a freeze authority and is not on the protocol allowlist")]
    FreezeAuthorityNotAllowed,
    #[msg("Signer is not the program's upgrade authority")]
    NotUpgradeAuthority,
}
