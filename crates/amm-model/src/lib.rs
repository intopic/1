//! Research-only integer virtual-reserve swap kernel.
//!
//! This models one constant-product interval, not a complete CLMM: no finite
//! price boundaries, tick crossings, actual token vaults, LP claims, or Solana
//! execution are implemented. Never infer real-vault solvency from virtual
//! reserves. Fees accrue separately; they are not added to active reserves.
//! Integer division rounds output down and the input fee up. Consequently the
//! virtual reserve product may grow by rounding dust rather than stay exact.

pub const FEE_DENOMINATOR: u128 = 1_000_000;
/// Deliberate research-model bound. Checked intermediates can still overflow;
/// such quotes return an error instead of using arbitrary-precision arithmetic.
pub const MAX_AMOUNT: u128 = (1u128 << 96) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    XtoY,
    YtoX,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    ZeroReserve,
    ZeroInput,
    AmountOutOfBounds,
    InvalidFee,
    ArithmeticOverflow,
    InsufficientNetInput,
    ZeroOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quote {
    pub amount_in: u128,
    pub fee: u128,
    pub net_input: u128,
    pub amount_out: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    virtual_x: u128,
    virtual_y: u128,
    accrued_fee_x: u128,
    accrued_fee_y: u128,
    fee_ppm: u32,
}

impl Band {
    pub fn new(virtual_x: u128, virtual_y: u128, fee_ppm: u32) -> Result<Self, Error> {
        if virtual_x == 0 || virtual_y == 0 {
            return Err(Error::ZeroReserve);
        }
        if virtual_x > MAX_AMOUNT || virtual_y > MAX_AMOUNT {
            return Err(Error::AmountOutOfBounds);
        }
        if u128::from(fee_ppm) >= FEE_DENOMINATOR {
            return Err(Error::InvalidFee);
        }
        Ok(Self {
            virtual_x,
            virtual_y,
            accrued_fee_x: 0,
            accrued_fee_y: 0,
            fee_ppm,
        })
    }

    pub fn virtual_reserves(&self) -> (u128, u128) {
        (self.virtual_x, self.virtual_y)
    }

    pub fn accrued_fees(&self) -> (u128, u128) {
        (self.accrued_fee_x, self.accrued_fee_y)
    }

    pub fn fee_ppm(&self) -> u32 {
        self.fee_ppm
    }

    /// Previews the same checks and rounding used by `swap_exact_in`.
    /// A quote is a snapshot; it is not an executable promise or state lock.
    pub fn quote_exact_in(&self, direction: Direction, amount_in: u128) -> Result<Quote, Error> {
        self.preview(direction, amount_in).map(|(quote, _)| quote)
    }

    /// Recomputes against current state; commits only after every check passes.
    /// Slippage enforcement and real token transfers belong to a future layer.
    pub fn swap_exact_in(&mut self, direction: Direction, amount_in: u128) -> Result<Quote, Error> {
        let (quote, next) = self.preview(direction, amount_in)?;
        *self = next;
        Ok(quote)
    }

    fn preview(&self, direction: Direction, amount_in: u128) -> Result<(Quote, Self), Error> {
        if amount_in == 0 {
            return Err(Error::ZeroInput);
        }
        if amount_in > MAX_AMOUNT {
            return Err(Error::AmountOutOfBounds);
        }
        let fee_numerator = amount_in
            .checked_mul(u128::from(self.fee_ppm))
            .ok_or(Error::ArithmeticOverflow)?;
        // Quotient + remainder avoids an overflowing numerator + denominator - 1.
        let fee = (fee_numerator / FEE_DENOMINATOR)
            .checked_add(u128::from(fee_numerator % FEE_DENOMINATOR != 0))
            .ok_or(Error::ArithmeticOverflow)?;
        let net_input = amount_in
            .checked_sub(fee)
            .ok_or(Error::ArithmeticOverflow)?;
        if net_input == 0 {
            return Err(Error::InsufficientNetInput);
        }
        let (reserve_in, reserve_out, accrued_fee_in) = match direction {
            Direction::XtoY => (self.virtual_x, self.virtual_y, self.accrued_fee_x),
            Direction::YtoX => (self.virtual_y, self.virtual_x, self.accrued_fee_y),
        };
        let next_in = reserve_in
            .checked_add(net_input)
            .ok_or(Error::ArithmeticOverflow)?;
        let output_numerator = reserve_out
            .checked_mul(net_input)
            .ok_or(Error::ArithmeticOverflow)?;
        let amount_out = output_numerator / next_in;
        if amount_out == 0 {
            return Err(Error::ZeroOutput);
        }
        let next_out = reserve_out
            .checked_sub(amount_out)
            .ok_or(Error::ArithmeticOverflow)?;
        let next_fee = accrued_fee_in
            .checked_add(fee)
            .ok_or(Error::ArithmeticOverflow)?;
        if next_in > MAX_AMOUNT || next_fee > MAX_AMOUNT {
            return Err(Error::AmountOutOfBounds);
        }
        let mut next = *self;
        match direction {
            Direction::XtoY => {
                next.virtual_x = next_in;
                next.virtual_y = next_out;
                next.accrued_fee_x = next_fee;
            }
            Direction::YtoX => {
                next.virtual_y = next_in;
                next.virtual_x = next_out;
                next.accrued_fee_y = next_fee;
            }
        }
        Ok((
            Quote {
                amount_in,
                fee,
                net_input,
                amount_out,
            },
            next,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_directions_conserve_input_and_separate_fees() {
        for direction in [Direction::XtoY, Direction::YtoX] {
            let mut band = Band::new(1_000_000, 1_000_000, 3_000).unwrap();
            let before = band;
            let quote = band.swap_exact_in(direction, 10_001).unwrap();
            assert_eq!(quote.fee, 31); // ceil(10_001 * .003)
            assert_eq!(quote.net_input + quote.fee, quote.amount_in);
            let (x, y) = band.virtual_reserves();
            let (fx, fy) = band.accrued_fees();
            match direction {
                Direction::XtoY => {
                    assert_eq!(x + fx - before.virtual_x, quote.amount_in);
                    assert_eq!(before.virtual_y - y, quote.amount_out);
                    assert_eq!(fy, 0);
                }
                Direction::YtoX => {
                    assert_eq!(y + fy - before.virtual_y, quote.amount_in);
                    assert_eq!(before.virtual_x - x, quote.amount_out);
                    assert_eq!(fx, 0);
                }
            }
            assert!(x * y >= before.virtual_x * before.virtual_y);
            assert!(x > 0 && y > 0);
        }
    }

    #[test]
    fn invalid_parameters_and_tiny_trades_are_rejected() {
        assert_eq!(Band::new(0, 1, 0), Err(Error::ZeroReserve));
        assert_eq!(
            Band::new(1, MAX_AMOUNT + 1, 0),
            Err(Error::AmountOutOfBounds)
        );
        assert_eq!(Band::new(1, 1, 1_000_000), Err(Error::InvalidFee));
        assert!(Band::new(1, 1, 999_999).is_ok());
        let band = Band::new(1_000, 1_000, 1).unwrap();
        assert_eq!(
            band.quote_exact_in(Direction::XtoY, 0),
            Err(Error::ZeroInput)
        );
        assert_eq!(
            band.quote_exact_in(Direction::XtoY, MAX_AMOUNT + 1),
            Err(Error::AmountOutOfBounds)
        );
        assert_eq!(
            band.quote_exact_in(Direction::XtoY, 1),
            Err(Error::InsufficientNetInput)
        );
        let tiny_output = Band::new(1_000_000, 1, 0).unwrap();
        assert_eq!(
            tiny_output.quote_exact_in(Direction::XtoY, 1),
            Err(Error::ZeroOutput)
        );
    }

    #[test]
    fn overflow_and_reserve_bounds_fail_without_partial_mutation() {
        let mut overflow = Band::new(1u128 << 80, 1u128 << 80, 0).unwrap();
        let before = overflow;
        assert_eq!(
            overflow.swap_exact_in(Direction::XtoY, 1u128 << 80),
            Err(Error::ArithmeticOverflow)
        );
        assert_eq!(overflow, before);
        let mut bounded = Band::new(MAX_AMOUNT, MAX_AMOUNT, 0).unwrap();
        let before = bounded;
        assert_eq!(
            bounded.swap_exact_in(Direction::XtoY, 2),
            Err(Error::AmountOutOfBounds)
        );
        assert_eq!(bounded, before);
    }

    #[test]
    fn proportional_routing_avoids_fragmentation_for_matching_shards() {
        let whole = Band::new(1_000_000, 1_000_000, 0).unwrap();
        let shard = Band::new(500_000, 500_000, 0).unwrap();
        let whole_out = whole
            .quote_exact_in(Direction::XtoY, 100_000)
            .unwrap()
            .amount_out;
        let one_shard_out = shard
            .quote_exact_in(Direction::XtoY, 100_000)
            .unwrap()
            .amount_out;
        let routed_out = 2 * shard
            .quote_exact_in(Direction::XtoY, 50_000)
            .unwrap()
            .amount_out;
        assert_eq!(whole_out, 90_909);
        assert_eq!(one_shard_out, 83_333);
        // Separate output floors can lose at most one raw token unit here.
        assert!(routed_out <= whole_out && whole_out - routed_out <= 1);
        assert!(one_shard_out < routed_out);
    }

    #[test]
    fn stale_quotes_do_not_define_a_shared_pool_execution() {
        let mut band = Band::new(1_000, 1_000, 0).unwrap();
        let stale = band.quote_exact_in(Direction::XtoY, 100).unwrap();
        let first = band.swap_exact_in(Direction::XtoY, 100).unwrap();
        let second = band.swap_exact_in(Direction::XtoY, 100).unwrap();
        assert_eq!(stale.amount_out, first.amount_out);
        assert!(second.amount_out < stale.amount_out);
        assert!(2 * stale.amount_out > first.amount_out + second.amount_out);
    }
}
