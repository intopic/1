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
}
