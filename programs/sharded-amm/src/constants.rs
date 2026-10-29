//! Protocol constants that are part of the ABI. Changing any value here changes
//! account addresses or registry semantics and requires a schema version bump.

/// Account schema version, also the `schema_u16_le` market seed component.
pub const SCHEMA_VERSION: u16 = 1;

/// Shards funded and activated atomically when the market opens (IDs 0..=7).
pub const INITIAL_SHARDS: u8 = 8;
/// Registry capacity: valid shard IDs are 0..=55.
pub const MAX_SHARDS: u8 = 56;
/// `0x0000_0000_0000_00ff`.
pub const INITIAL_MASK: u64 = (1u64 << INITIAL_SHARDS) - 1;
/// `0x00ff_ffff_ffff_ffff`; bits 56..=63 are reserved and must stay zero.
pub const VALID_MASK: u64 = (1u64 << MAX_SHARDS) - 1;
pub const RESERVED_MASK: u64 = !VALID_MASK;

const _: () = assert!(INITIAL_SHARDS > 0 && INITIAL_SHARDS <= MAX_SHARDS);
const _: () = assert!(MAX_SHARDS < 64);
const _: () = assert!(INITIAL_MASK & RESERVED_MASK == 0);

// PDA seed prefixes. Full layouts are normative in docs/account-layout.md.
pub const MARKET_SEED: &[u8] = b"market";
pub const REGISTRY_SEED: &[u8] = b"registry";
pub const SHARD_SEED: &[u8] = b"shard";
pub const VAULT_AUTH_SEED: &[u8] = b"vault_auth";
pub const VAULT_SEED: &[u8] = b"vault";
pub const TICK_ARRAY_SEED: &[u8] = b"ticks";
pub const POSITION_SEED: &[u8] = b"lp_pos";

// Market parameter bounds (docs/numeric-spec.md section 1, ADR-0004).
pub const FEE_RATE_DENOMINATOR: u32 = 1_000_000;
pub const MAX_FEE_RATE_PPM: u32 = 100_000;
pub const PROTOCOL_SHARE_DENOMINATOR: u16 = 10_000;
pub const MAX_PROTOCOL_SHARE_BPS: u16 = 5_000;
pub const MAX_TICK_SPACING: u16 = 1_000;
pub const MIN_TICK: i32 = -443_636;
pub const MAX_TICK: i32 = 443_636;

/// Initialized-tick slots per `TickArray`; an array spans `32 * tick_spacing` ticks.
pub const TICKS_PER_ARRAY: usize = 32;
/// Upper bound on tick arrays a single swap may traverse (ADR-0004).
pub const MAX_SWAP_TICK_ARRAYS: usize = 3;
/// Hard cap on swap loop iterations; exceeding it fails instead of looping.
pub const MAX_SWAP_STEPS: u32 = 128;
/// Delay between proposing and applying a fee change (ADR-0004: admin delay).
pub const FEE_TIMELOCK_SECONDS: i64 = 86_400;
/// Ceiling of a dynamic fee, 0.80% (ADR-0005). The base fee may be higher
/// only when the dynamic fee is off.
pub const MAX_DYNAMIC_FEE_RATE_PPM: u32 = 8_000;
/// Volatility accumulator units per `tick_spacing` moved (ADR-0005).
pub const VOL_UNIT: u64 = 1_000;
/// Upper bound on `max_vol_accumulator`: 100 000 spacings.
pub const MAX_VOL_ACCUMULATOR: u32 = 100_000_000;
/// Upper bound on the volatility and load coefficients, so no term can
/// overflow `u64` before the clamp.
pub const MAX_DYNAMIC_COEFF_PPM: u32 = 1_000_000;
/// Upper bound on the activation price tolerance.
pub const MAX_ACTIVATION_DEVIATION_BPS: u16 = 10_000;
