use crate::errors::AmmError;
use crate::math::tick_math::{check_sqrt_price, tick_at_sqrt_price, MAX_SQRT_PRICE_X64};
use crate::math::wide::U256;
use crate::state::ShardState;

/// The mutable CLMM part of a `ShardState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pool {
    pub sqrt_price: u128,
    pub tick: i32,
    pub liquidity: u128,
    pub growth_a: U256,
    pub growth_b: U256,
    pub protocol_a: u64,
    pub protocol_b: u64,
}

impl Pool {
    /// A fresh pool at `sqrt_price`; the price must be in `[MIN, MAX)`.
    pub fn at_price(sqrt_price: u128) -> Result<Self, AmmError> {
        check_sqrt_price(sqrt_price)?;
        if sqrt_price == MAX_SQRT_PRICE_X64 {
            return Err(AmmError::SqrtPriceOutOfRange);
        }
        Ok(Self {
            sqrt_price,
            tick: tick_at_sqrt_price(sqrt_price)?,
            liquidity: 0,
            growth_a: U256::zero(),
            growth_b: U256::zero(),
            protocol_a: 0,
            protocol_b: 0,
        })
    }

    pub fn load(s: &ShardState) -> Result<Self, AmmError> {
        if s.sqrt_price_x64 == 0 {
            return Err(AmmError::PriceNotInitialized);
        }
        Ok(Self {
            sqrt_price: s.sqrt_price_x64,
            tick: s.tick_current,
            liquidity: s.liquidity,
            growth_a: U256(s.fee_growth_global_a_x128),
            growth_b: U256(s.fee_growth_global_b_x128),
            protocol_a: s.protocol_fees_a,
            protocol_b: s.protocol_fees_b,
        })
    }

    pub fn store(&self, s: &mut ShardState) {
        s.sqrt_price_x64 = self.sqrt_price;
        s.tick_current = self.tick;
        s.liquidity = self.liquidity;
        s.fee_growth_global_a_x128 = self.growth_a.0;
        s.fee_growth_global_b_x128 = self.growth_b.0;
        s.protocol_fees_a = self.protocol_a;
        s.protocol_fees_b = self.protocol_b;
    }
}
