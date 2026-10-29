//! `ProtocolConfig`: the one account that holds the protocol's roles and
//! the rules for new markets (D-04, D-05, D-07 of the mainnet analysis).
//!
//! PDA `["protocol"]`, created once by the program's upgrade authority.
//! Markets never take roles from their creator: `initialize_market` copies
//! the protocol's roles into the market, and every privileged market
//! instruction requires `market.roles_version == protocol.roles_version`, so
//! a rotation takes effect everywhere at once. `sync_market_roles` (open to
//! anyone) copies the new roles into a market; until then that market's
//! privileged instructions fail, and swaps and LP operations never read
//! this account.
//!
//! Every change is timelocked (`FEE_TIMELOCK_SECONDS`) except removing the
//! pauser, which the admin can do at once. A new admin must accept the role
//! with its own signature after the delay.

use anchor_lang::prelude::*;

use crate::constants::{FEE_TIMELOCK_SECONDS, MAX_FREEZE_ALLOWLIST, MAX_PROFILES};
use crate::errors::AmmError;
use crate::state::{DynamicFeeParams, MarketParams};

#[account]
#[derive(InitSpace, Debug, PartialEq, Eq)]
pub struct ProtocolConfig {
    pub schema_version: u16,
    pub bump: u8,
    pub roles: Roles,
    /// Admin chosen by an applied role change, waiting for `accept_admin`.
    pub pending_admin: Pubkey,
    /// Bumped by every role change; markets must match it.
    pub roles_version: u32,
    pub pending_roles: PendingRoles,
    pub settings: ProtocolSettings,
    pub pending_settings: PendingSettings,
    /// Zeroed space for future fields without a realloc.
    pub reserved: [u8; 64],
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct Roles {
    /// Parameters within bounds, behind the timelock. Cannot move principal.
    pub admin: Pubkey,
    /// Prepares and activates shards when on-chain readiness checks pass.
    pub keeper: Pubkey,
    /// Pauses risky operations; never blocks LP exits. May be empty.
    pub pauser: Pubkey,
    /// Treasury: the only key that collects protocol fees.
    pub revenue_authority: Pubkey,
}

impl Roles {
    /// Admin, keeper and revenue authority must be set; the pauser may be
    /// empty (removed in an emergency).
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        if self.admin == Pubkey::default()
            || self.keeper == Pubkey::default()
            || self.revenue_authority == Pubkey::default()
        {
            return Err(AmmError::Unauthorized);
        }
        Ok(())
    }
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct PendingRoles {
    pub roles: Roles,
    /// Earliest unix time at which the change can be applied.
    pub eta: i64,
    pub active: bool,
}

/// A market type the protocol allows: spacing, fees and the dynamic fee a
/// new market starts with. Its creator only picks which profile.
#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct MarketProfile {
    pub enabled: bool,
    pub tick_spacing: u16,
    pub fee_rate_ppm: u32,
    pub protocol_share_bps: u16,
    pub dynamic: DynamicFeeParams,
}

impl MarketProfile {
    pub fn params(&self) -> MarketParams {
        MarketParams {
            fee_rate_ppm: self.fee_rate_ppm,
            protocol_share_bps: self.protocol_share_bps,
            tick_spacing: self.tick_spacing,
        }
    }

    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        if !self.enabled {
            return Ok(());
        }
        self.params().validate()?;
        self.dynamic.validate(self.fee_rate_ppm)
    }
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct ProtocolSettings {
    pub profiles: [MarketProfile; MAX_PROFILES],
    /// Lamports a market's creator pays the treasury. Charged from the
    /// finalize phase on; stored now so the layout does not change then.
    pub creation_fee_lamports: u64,
    /// Mints that may keep a freeze authority (regulated stablecoins such
    /// as USDC and USDT). Any other mint with one is rejected (D-07).
    pub freeze_allowlist: [Pubkey; MAX_FREEZE_ALLOWLIST],
}

impl ProtocolSettings {
    pub fn validate(&self) -> std::result::Result<(), AmmError> {
        for p in &self.profiles {
            p.validate()?;
        }
        let list = &self.freeze_allowlist;
        for (i, k) in list.iter().enumerate() {
            if *k != Pubkey::default() && list[..i].contains(k) {
                return Err(AmmError::InvalidProtocolSettings);
            }
        }
        Ok(())
    }

    pub fn profile(&self, index: u8) -> std::result::Result<MarketProfile, AmmError> {
        match self.profiles.get(usize::from(index)) {
            Some(p) if p.enabled => Ok(*p),
            _ => Err(AmmError::ProfileNotAllowed),
        }
    }

    /// A mint with this freeze authority may be listed.
    pub fn freeze_allowed(&self, mint: &Pubkey, freeze_authority: Option<&Pubkey>) -> bool {
        freeze_authority.is_none()
            || (*mint != Pubkey::default() && self.freeze_allowlist.contains(mint))
    }
}

#[derive(
    AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub struct PendingSettings {
    pub settings: ProtocolSettings,
    pub eta: i64,
    pub active: bool,
}

fn eta(now: i64) -> std::result::Result<i64, AmmError> {
    now.checked_add(FEE_TIMELOCK_SECONDS)
        .ok_or(AmmError::MathOverflow)
}

impl ProtocolConfig {
    fn only_admin(&self, signer: &Pubkey) -> std::result::Result<(), AmmError> {
        if *signer != self.roles.admin {
            return Err(AmmError::Unauthorized);
        }
        Ok(())
    }

    fn bump_version(&mut self) {
        self.roles_version = self.roles_version.wrapping_add(1);
    }

    /// Admin proposes new roles. A different admin is not installed by
    /// `apply_roles`; it becomes `pending_admin` and must `accept_admin`.
    /// A new proposal replaces a pending one and restarts the delay.
    pub fn propose_roles(
        &mut self,
        signer: &Pubkey,
        roles: Roles,
        now: i64,
    ) -> std::result::Result<PendingRoles, AmmError> {
        self.only_admin(signer)?;
        roles.validate()?;
        self.pending_roles = PendingRoles {
            roles,
            eta: eta(now)?,
            active: true,
        };
        Ok(self.pending_roles)
    }

    /// Applies the pending roles once the delay has passed (admin only).
    /// Returns the previous roles.
    pub fn apply_roles(
        &mut self,
        signer: &Pubkey,
        now: i64,
    ) -> std::result::Result<Roles, AmmError> {
        self.only_admin(signer)?;
        let p = self.pending_roles;
        if !p.active {
            return Err(AmmError::NoPendingChange);
        }
        if now < p.eta {
            return Err(AmmError::TimelockNotElapsed);
        }
        let old = self.roles;
        self.roles = Roles {
            admin: old.admin,
            ..p.roles
        };
        self.pending_admin = if p.roles.admin == old.admin {
            Pubkey::default()
        } else {
            p.roles.admin
        };
        self.pending_roles = PendingRoles::default();
        self.bump_version();
        Ok(old)
    }

    pub fn cancel_roles_change(&mut self, signer: &Pubkey) -> std::result::Result<(), AmmError> {
        self.only_admin(signer)?;
        if !self.pending_roles.active && self.pending_admin == Pubkey::default() {
            return Err(AmmError::NoPendingChange);
        }
        self.pending_roles = PendingRoles::default();
        self.pending_admin = Pubkey::default();
        Ok(())
    }

    /// The pending admin takes over with its own signature (second step).
    pub fn accept_admin(&mut self, signer: &Pubkey) -> std::result::Result<Pubkey, AmmError> {
        if self.pending_admin == Pubkey::default() {
            return Err(AmmError::NoPendingChange);
        }
        if *signer != self.pending_admin {
            return Err(AmmError::Unauthorized);
        }
        let old = self.roles.admin;
        self.roles.admin = self.pending_admin;
        self.pending_admin = Pubkey::default();
        self.bump_version();
        Ok(old)
    }

    /// Emergency: the admin removes the pauser at once (no timelock), e.g.
    /// when its key leaks. Returns the removed key.
    pub fn remove_pauser(&mut self, signer: &Pubkey) -> std::result::Result<Pubkey, AmmError> {
        self.only_admin(signer)?;
        let old = self.roles.pauser;
        if old == Pubkey::default() {
            return Err(AmmError::NoPendingChange);
        }
        self.roles.pauser = Pubkey::default();
        if self.pending_roles.roles.pauser == old {
            self.pending_roles.roles.pauser = Pubkey::default();
        }
        self.bump_version();
        Ok(old)
    }

    pub fn propose_settings(
        &mut self,
        signer: &Pubkey,
        settings: ProtocolSettings,
        now: i64,
    ) -> std::result::Result<PendingSettings, AmmError> {
        self.only_admin(signer)?;
        settings.validate()?;
        self.pending_settings = PendingSettings {
            settings,
            eta: eta(now)?,
            active: true,
        };
        Ok(self.pending_settings)
    }

    pub fn apply_settings(
        &mut self,
        signer: &Pubkey,
        now: i64,
    ) -> std::result::Result<ProtocolSettings, AmmError> {
        self.only_admin(signer)?;
        let p = self.pending_settings;
        if !p.active {
            return Err(AmmError::NoPendingChange);
        }
        if now < p.eta {
            return Err(AmmError::TimelockNotElapsed);
        }
        let old = self.settings;
        self.settings = p.settings;
        self.pending_settings = PendingSettings::default();
        Ok(old)
    }

    pub fn cancel_settings_change(&mut self, signer: &Pubkey) -> std::result::Result<(), AmmError> {
        self.only_admin(signer)?;
        if !self.pending_settings.active {
            return Err(AmmError::NoPendingChange);
        }
        self.pending_settings = PendingSettings::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn roles(base: u8) -> Roles {
        Roles {
            admin: key(base),
            keeper: key(base + 1),
            pauser: key(base + 2),
            revenue_authority: key(base + 3),
        }
    }

    fn config() -> ProtocolConfig {
        ProtocolConfig {
            schema_version: 1,
            bump: 255,
            roles: roles(1),
            pending_admin: Pubkey::default(),
            roles_version: 0,
            pending_roles: PendingRoles::default(),
            settings: ProtocolSettings::default(),
            pending_settings: PendingSettings::default(),
            reserved: [0; 64],
        }
    }

    #[test]
    fn role_rotation_is_timelocked_and_admin_needs_acceptance() {
        let mut c = config();
        let (admin, keeper) = (key(1), key(2));
        let new = roles(10);
        assert_eq!(
            c.propose_roles(&keeper, new, 0),
            Err(AmmError::Unauthorized)
        );
        assert_eq!(
            c.propose_roles(
                &admin,
                Roles {
                    keeper: Pubkey::default(),
                    ..new
                },
                0
            ),
            Err(AmmError::Unauthorized)
        );
        c.propose_roles(&admin, new, 1_000).unwrap();
        assert_eq!(
            c.apply_roles(&admin, 1_000 + FEE_TIMELOCK_SECONDS - 1),
            Err(AmmError::TimelockNotElapsed)
        );
        assert_eq!(c.roles, roles(1));
        let old = c.apply_roles(&admin, 1_000 + FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(old, roles(1));
        // Keeper, pauser and treasury change now; the admin only after the
        // new key signs.
        assert_eq!(c.roles, Roles { admin, ..new });
        assert_eq!(c.pending_admin, new.admin);
        assert_eq!(c.roles_version, 1);
        assert_eq!(c.accept_admin(&admin), Err(AmmError::Unauthorized));
        assert_eq!(c.accept_admin(&new.admin), Ok(admin));
        assert_eq!(c.roles, new);
        assert_eq!(c.roles_version, 2);
        assert_eq!(c.accept_admin(&new.admin), Err(AmmError::NoPendingChange));
        // The old admin has no power left.
        assert_eq!(
            c.propose_roles(&admin, roles(1), 0),
            Err(AmmError::Unauthorized)
        );
    }

    #[test]
    fn same_admin_needs_no_acceptance_and_changes_can_be_cancelled() {
        let mut c = config();
        let admin = key(1);
        let new = Roles { admin, ..roles(20) };
        c.propose_roles(&admin, new, 0).unwrap();
        c.cancel_roles_change(&admin).unwrap();
        assert_eq!(
            c.apply_roles(&admin, i64::MAX),
            Err(AmmError::NoPendingChange)
        );
        c.propose_roles(&admin, new, 0).unwrap();
        c.apply_roles(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(c.roles, new);
        assert_eq!(c.pending_admin, Pubkey::default());
        // A pending admin can be withdrawn before it accepts.
        c.propose_roles(&admin, roles(30), 0).unwrap();
        c.apply_roles(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        c.cancel_roles_change(&admin).unwrap();
        assert_eq!(c.accept_admin(&key(30)), Err(AmmError::NoPendingChange));
        assert_eq!(c.roles.admin, admin);
        assert_eq!(
            c.propose_roles(&admin, new, i64::MAX),
            Err(AmmError::MathOverflow)
        );
    }

    #[test]
    fn admin_removes_the_pauser_at_once() {
        let mut c = config();
        let (admin, pauser) = (key(1), key(3));
        assert_eq!(c.remove_pauser(&pauser), Err(AmmError::Unauthorized));
        // A pending change that would keep the leaked key loses it too.
        c.propose_roles(&admin, Roles { admin, ..roles(1) }, 0)
            .unwrap();
        assert_eq!(c.remove_pauser(&admin), Ok(pauser));
        assert_eq!(c.roles.pauser, Pubkey::default());
        assert_eq!(c.roles_version, 1);
        c.apply_roles(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(c.roles.pauser, Pubkey::default());
        assert_eq!(c.remove_pauser(&admin), Err(AmmError::NoPendingChange));
    }

    #[test]
    fn settings_are_timelocked_and_validated() {
        let mut c = config();
        let admin = key(1);
        let mut s = ProtocolSettings::default();
        s.profiles[0] = MarketProfile {
            enabled: true,
            tick_spacing: 10,
            fee_rate_ppm: 500,
            protocol_share_bps: 1600,
            dynamic: DynamicFeeParams::disabled(500),
        };
        s.freeze_allowlist[0] = key(50);
        let mut bad = s;
        bad.profiles[1] = MarketProfile {
            tick_spacing: 0,
            ..s.profiles[0]
        };
        assert_eq!(
            c.propose_settings(&admin, bad, 0),
            Err(AmmError::InvalidTickSpacing)
        );
        let mut dup = s;
        dup.freeze_allowlist[1] = key(50);
        assert_eq!(
            c.propose_settings(&admin, dup, 0),
            Err(AmmError::InvalidProtocolSettings)
        );
        assert_eq!(
            c.propose_settings(&key(2), s, 0),
            Err(AmmError::Unauthorized)
        );
        c.propose_settings(&admin, s, 0).unwrap();
        assert_eq!(
            c.apply_settings(&admin, FEE_TIMELOCK_SECONDS - 1),
            Err(AmmError::TimelockNotElapsed)
        );
        c.apply_settings(&admin, FEE_TIMELOCK_SECONDS).unwrap();
        assert_eq!(c.settings, s);
        assert_eq!(c.settings.profile(0), Ok(s.profiles[0]));
        assert_eq!(c.settings.profile(1), Err(AmmError::ProfileNotAllowed));
        assert_eq!(c.settings.profile(200), Err(AmmError::ProfileNotAllowed));
        // Freeze authority: allowed only for listed mints.
        assert!(c.settings.freeze_allowed(&key(9), None));
        assert!(c.settings.freeze_allowed(&key(50), Some(&key(7))));
        assert!(!c.settings.freeze_allowed(&key(9), Some(&key(7))));
        assert!(!c.settings.freeze_allowed(&Pubkey::default(), Some(&key(7))));
        c.propose_settings(&admin, ProtocolSettings::default(), 0)
            .unwrap();
        c.cancel_settings_change(&admin).unwrap();
        assert_eq!(
            c.apply_settings(&admin, i64::MAX),
            Err(AmmError::NoPendingChange)
        );
    }
}
