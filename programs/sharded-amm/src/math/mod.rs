//! Pure arithmetic and bit manipulation. Nothing here reads accounts, clocks
//! or RPC state, so every function is testable on the host.
pub mod bitmap;
pub mod fee_math;
pub mod liquidity_math;
pub mod sqrt_price_math;
pub mod swap_step;
pub mod tick_math;
mod tick_table;
pub mod wide;
