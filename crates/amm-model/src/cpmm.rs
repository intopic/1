//! CPMM baseline with LP shares and a vault ledger (research T04).
//!
//! This is the comparison baseline for the CLMM benchmarks, not a CLMM and not
//! deployed. Reserves and vault balances are tracked separately, so a direct
//! donation to the vault changes neither the share price nor the reserves.
//! LP fees stay in the reserves (they raise the value of every share);
//! protocol fees accrue outside the reserves and are collected separately.
//!
//! Rounding favors the pool everywhere: shares minted and amounts paid out
//! round down, amounts taken from depositors round up. `MINIMUM_SHARES` are
//! locked at the first deposit so the first depositor cannot inflate the
//! share price to steal later deposits.

pub const FEE_DENOMINATOR: u64 = 1_000_000;
pub const PROTOCOL_SHARE_DENOMINATOR: u64 = 10_000;
pub const MINIMUM_SHARES: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    ZeroAmount,
    InvalidFee,
    InvalidProtocolShare,
    Overflow,
    InsufficientLiquidity,
    ZeroShares,
    ZeroOutput,
    NotEnoughShares,
    SlippageExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ledger {
    pub vault_x: u64,
    pub vault_y: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deposit {
    pub shares: u64,
    pub used_x: u64,
    pub used_y: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swap {
    pub amount_in: u64,
    pub fee: u64,
    pub protocol_fee: u64,
    pub amount_out: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pool {
    pub reserve_x: u64,
    pub reserve_y: u64,
    pub total_shares: u64,
    pub protocol_x: u64,
    pub protocol_y: u64,
    pub vault: Ledger,
    fee_ppm: u32,
    protocol_share_bps: u16,
}

fn mul_div(a: u64, b: u64, d: u64, round_up: bool) -> Result<u64, Error> {
    if d == 0 {
        return Err(Error::InsufficientLiquidity);
    }
    let n = u128::from(a) * u128::from(b);
    let d = u128::from(d);
    let q = if round_up { n.div_ceil(d) } else { n / d };
    u64::try_from(q).map_err(|_| Error::Overflow)
}

impl Pool {
    pub fn new(fee_ppm: u32, protocol_share_bps: u16) -> Result<Self, Error> {
        if u64::from(fee_ppm) > FEE_DENOMINATOR / 10 {
            return Err(Error::InvalidFee);
        }
        if u64::from(protocol_share_bps) > PROTOCOL_SHARE_DENOMINATOR {
            return Err(Error::InvalidProtocolShare);
        }
        Ok(Self {
            reserve_x: 0,
            reserve_y: 0,
            total_shares: 0,
            protocol_x: 0,
            protocol_y: 0,
            vault: Ledger::default(),
            fee_ppm,
            protocol_share_bps,
        })
    }

    /// Adds liquidity using at most `max_x`/`max_y`; returns what was used.
    pub fn deposit(&mut self, max_x: u64, max_y: u64, min_shares: u64) -> Result<Deposit, Error> {
        if max_x == 0 || max_y == 0 {
            return Err(Error::ZeroAmount);
        }
        let mut next = *self;
        let d = if self.total_shares == 0 {
            let root = (u128::from(max_x) * u128::from(max_y)).isqrt() as u64;
            let shares = root
                .checked_sub(MINIMUM_SHARES)
                .filter(|&s| s > 0)
                .ok_or(Error::ZeroShares)?;
            next.total_shares = root; // MINIMUM_SHARES are minted to nobody
            Deposit {
                shares,
                used_x: max_x,
                used_y: max_y,
            }
        } else {
            let sx = mul_div(max_x, self.total_shares, self.reserve_x, false)?;
            let sy = mul_div(max_y, self.total_shares, self.reserve_y, false)?;
            let shares = sx.min(sy);
            if shares == 0 {
                return Err(Error::ZeroShares);
            }
            let used_x = mul_div(shares, self.reserve_x, self.total_shares, true)?;
            let used_y = mul_div(shares, self.reserve_y, self.total_shares, true)?;
            next.total_shares = self
                .total_shares
                .checked_add(shares)
                .ok_or(Error::Overflow)?;
            Deposit {
                shares,
                used_x,
                used_y,
            }
        };
        if d.shares < min_shares {
            return Err(Error::SlippageExceeded);
        }
        next.reserve_x = self
            .reserve_x
            .checked_add(d.used_x)
            .ok_or(Error::Overflow)?;
        next.reserve_y = self
            .reserve_y
            .checked_add(d.used_y)
            .ok_or(Error::Overflow)?;
        next.vault.vault_x = self
            .vault
            .vault_x
            .checked_add(d.used_x)
            .ok_or(Error::Overflow)?;
        next.vault.vault_y = self
            .vault
            .vault_y
            .checked_add(d.used_y)
            .ok_or(Error::Overflow)?;
        *self = next;
        Ok(d)
    }

    /// Burns `shares` and pays the pro-rata reserves, rounded down.
    pub fn withdraw(&mut self, shares: u64) -> Result<(u64, u64), Error> {
        if shares == 0 {
            return Err(Error::ZeroAmount);
        }
        // Locked shares can never be burned, so total - MINIMUM is the cap.
        if shares > self.total_shares.saturating_sub(MINIMUM_SHARES) {
            return Err(Error::NotEnoughShares);
        }
        let x = mul_div(shares, self.reserve_x, self.total_shares, false)?;
        let y = mul_div(shares, self.reserve_y, self.total_shares, false)?;
        self.total_shares -= shares;
        self.reserve_x -= x;
        self.reserve_y -= y;
        self.vault.vault_x -= x;
        self.vault.vault_y -= y;
        Ok((x, y))
    }

    pub fn swap_exact_in(
        &mut self,
        side_in: Side,
        amount_in: u64,
        min_out: u64,
    ) -> Result<Swap, Error> {
        if amount_in == 0 {
            return Err(Error::ZeroAmount);
        }
        let (r_in, r_out) = match side_in {
            Side::X => (self.reserve_x, self.reserve_y),
            Side::Y => (self.reserve_y, self.reserve_x),
        };
        if r_in == 0 || r_out == 0 {
            return Err(Error::InsufficientLiquidity);
        }
        let fee = mul_div(amount_in, u64::from(self.fee_ppm), FEE_DENOMINATOR, true)?;
        let protocol_fee = mul_div(
            fee,
            u64::from(self.protocol_share_bps),
            PROTOCOL_SHARE_DENOMINATOR,
            false,
        )?;
        let net = amount_in - fee;
        let new_in = r_in.checked_add(net).ok_or(Error::Overflow)?;
        let amount_out = mul_div(r_out, net, new_in, false)?;
        if amount_out == 0 {
            return Err(Error::ZeroOutput);
        }
        if amount_out < min_out {
            return Err(Error::SlippageExceeded);
        }
        // The LP part of the fee joins the reserves after the price is set.
        let r_in_after = new_in
            .checked_add(fee - protocol_fee)
            .ok_or(Error::Overflow)?;
        let r_out_after = r_out - amount_out;
        match side_in {
            Side::X => {
                self.vault.vault_x = self
                    .vault
                    .vault_x
                    .checked_add(amount_in)
                    .ok_or(Error::Overflow)?;
                self.vault.vault_y -= amount_out;
                self.reserve_x = r_in_after;
                self.reserve_y = r_out_after;
                self.protocol_x += protocol_fee;
            }
            Side::Y => {
                self.vault.vault_y = self
                    .vault
                    .vault_y
                    .checked_add(amount_in)
                    .ok_or(Error::Overflow)?;
                self.vault.vault_x -= amount_out;
                self.reserve_y = r_in_after;
                self.reserve_x = r_out_after;
                self.protocol_y += protocol_fee;
            }
        }
        Ok(Swap {
            amount_in,
            fee,
            protocol_fee,
            amount_out,
        })
    }

    /// A transfer straight into the vault; it is not added to the reserves.
    pub fn donate(&mut self, side: Side, amount: u64) -> Result<(), Error> {
        let v = match side {
            Side::X => &mut self.vault.vault_x,
            Side::Y => &mut self.vault.vault_y,
        };
        *v = v.checked_add(amount).ok_or(Error::Overflow)?;
        Ok(())
    }

    pub fn collect_protocol(&mut self) -> (u64, u64) {
        let out = (self.protocol_x, self.protocol_y);
        self.vault.vault_x -= self.protocol_x;
        self.vault.vault_y -= self.protocol_y;
        self.protocol_x = 0;
        self.protocol_y = 0;
        out
    }

    /// Vaults always cover reserves plus uncollected protocol fees.
    pub fn check_ledger(&self) -> bool {
        u128::from(self.vault.vault_x) >= u128::from(self.reserve_x) + u128::from(self.protocol_x)
            && u128::from(self.vault.vault_y)
                >= u128::from(self.reserve_y) + u128::from(self.protocol_y)
            && (self.total_shares == 0 || self.total_shares >= MINIMUM_SHARES)
            && (self.total_shares == 0) == (self.reserve_x == 0 && self.reserve_y == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn amount(&mut self, max_bits: u32) -> u64 {
            let bits = 1 + (self.next() % u64::from(max_bits)) as u32;
            self.next() >> (64 - bits)
        }
    }

    fn cases() -> u64 {
        std::env::var("AMM_STRESS_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2_000)
    }

    #[test]
    fn first_deposit_locks_minimum_shares() {
        let mut p = Pool::new(3000, 2000).unwrap();
        assert_eq!(p.deposit(1_000, 1_000, 0), Err(Error::ZeroShares));
        let d = p.deposit(1_000_000, 4_000_000, 0).unwrap();
        assert_eq!(d.shares, 2_000_000 - MINIMUM_SHARES);
        assert_eq!(p.withdraw(d.shares + 1), Err(Error::NotEnoughShares));
        let (x, y) = p.withdraw(d.shares).unwrap();
        assert!(x < 1_000_000 && y < 4_000_000 && p.reserve_x > 0 && p.reserve_y > 0);
        assert!(p.check_ledger());
    }

    /// Classic inflation attack: tiny first deposit, then a large donation so
    /// the next depositor's shares round to zero. Donations do not count as
    /// reserves here, so the victim gets fair shares.
    #[test]
    fn donation_cannot_inflate_share_price() {
        let mut p = Pool::new(3000, 0).unwrap();
        let attacker = p.deposit(1_001, 1_001, 0).unwrap();
        assert_eq!(attacker.shares, 1);
        p.donate(Side::X, 1_000_000_000).unwrap();
        p.donate(Side::Y, 1_000_000_000).unwrap();
        let victim = p.deposit(1_000_000, 1_000_000, 0).unwrap();
        assert_eq!(victim.shares, 1_000_000 * 1_001 / 1_001);
        let (x, y) = p.withdraw(victim.shares).unwrap();
        assert!(x >= 999_999 && y >= 999_999);
        assert!(p.check_ledger());
    }

    #[test]
    fn swap_fees_split_and_k_grows() {
        let mut p = Pool::new(3000, 2000).unwrap();
        p.deposit(1_000_000_000, 1_000_000_000, 0).unwrap();
        let k0 = u128::from(p.reserve_x) * u128::from(p.reserve_y);
        let s = p.swap_exact_in(Side::X, 1_000_000, 0).unwrap();
        assert_eq!(s.fee, 3_000);
        assert_eq!(s.protocol_fee, 600);
        assert!(u128::from(p.reserve_x) * u128::from(p.reserve_y) > k0);
        assert_eq!(
            p.swap_exact_in(Side::Y, 1_000_000, u64::MAX),
            Err(Error::SlippageExceeded)
        );
        assert_eq!(p.collect_protocol(), (600, 0));
        assert!(p.check_ledger());
    }

    /// Random multi-LP sequences: the ledger always holds, k never falls on a
    /// swap, nobody profits from an immediate deposit/withdraw round trip, and
    /// the protocol plus remaining reserves account for every token.
    #[test]
    fn randomized_sequences_keep_the_ledger() {
        let mut r = Rng(0x1234_5678_9abc_def1);
        for case in 0..cases() {
            let fee = [0, 500, 1000, 3000, 100_000][(r.next() % 5) as usize];
            let mut p = Pool::new(fee, (r.next() % 5001) as u16).unwrap();
            let mut shares = [0u64; 4];
            let (mut paid_in_x, mut paid_in_y, mut out_x, mut out_y) = (0u128, 0u128, 0u128, 0u128);
            for _ in 0..40 {
                let who = (r.next() % 4) as usize;
                match r.next() % 5 {
                    0 | 1 => {
                        let (ax, ay) = (r.amount(50), r.amount(50));
                        let before = p;
                        if let Ok(d) = p.deposit(ax, ay, 0) {
                            assert!(d.used_x <= ax && d.used_y <= ay);
                            shares[who] += d.shares;
                            paid_in_x += u128::from(d.used_x);
                            paid_in_y += u128::from(d.used_y);
                            // Immediate exit returns no more than was put in.
                            let mut probe = p;
                            if let Ok((x, y)) = probe.withdraw(d.shares) {
                                assert!(x <= d.used_x && y <= d.used_y, "case {case}");
                            }
                        } else {
                            assert_eq!(p, before);
                        }
                    }
                    2 if shares[who] > 0 => {
                        let n = 1 + r.next() % shares[who];
                        if let Ok((x, y)) = p.withdraw(n) {
                            shares[who] -= n;
                            out_x += u128::from(x);
                            out_y += u128::from(y);
                        }
                    }
                    3 => {
                        let side = if r.next().is_multiple_of(2) {
                            Side::X
                        } else {
                            Side::Y
                        };
                        let k0 = u128::from(p.reserve_x) * u128::from(p.reserve_y);
                        let amt = r.amount(45);
                        if let Ok(s) = p.swap_exact_in(side, amt, 0) {
                            assert!(u128::from(p.reserve_x) * u128::from(p.reserve_y) >= k0);
                            match side {
                                Side::X => {
                                    paid_in_x += u128::from(s.amount_in);
                                    out_y += u128::from(s.amount_out);
                                }
                                Side::Y => {
                                    paid_in_y += u128::from(s.amount_in);
                                    out_x += u128::from(s.amount_out);
                                }
                            }
                        }
                    }
                    _ => {
                        let (x, y) = p.collect_protocol();
                        out_x += u128::from(x);
                        out_y += u128::from(y);
                    }
                }
                assert!(p.check_ledger(), "case {case}");
                // Token conservation (no donations in this test).
                assert_eq!(paid_in_x - out_x, u128::from(p.vault.vault_x));
                assert_eq!(paid_in_y - out_y, u128::from(p.vault.vault_y));
            }
            // The last LPs can always exit; only the locked minimum remains.
            for s in shares.iter_mut().filter(|s| **s > 0) {
                p.withdraw(*s).unwrap();
                *s = 0;
            }
            assert!(p.total_shares == 0 || p.total_shares == MINIMUM_SHARES);
        }
    }
}
