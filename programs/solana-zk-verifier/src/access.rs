//! Business admission policies and access grants, independent of proof freshness.
//! Consumers MUST pin their policy address when calling `require_access_grant`.
use anchor_lang::prelude::*;

use crate::{gate, AttestationRecord, VerifierState};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, InitSpace)]
pub struct AccessPolicyConfig {
    /// Zero disables this condition. Values use the trusted issuer's balance units.
    pub min_balance: u64,
    pub min_age: u8,
    /// Source-data age at admission/renewal, NOT the access lifetime.
    pub balance_max_age_secs: i64,
    /// Zero allows a stable DOB attestation without an age limit.
    pub age_max_age_secs: i64,
    /// Zero grants access without a scheduled expiry.
    pub access_duration_secs: i64,
    /// Zero checks conditions only at admission/renewal. Otherwise refresh is due
    /// after this interval; refresh does not extend the original access expiry.
    pub recheck_interval_secs: i64,
    pub balance_source: Option<[u8; 32]>,
    pub age_source: Option<[u8; 32]>,
}

impl AccessPolicyConfig {
    pub fn validate(&self) -> Result<()> {
        require!(
            self.balance_max_age_secs >= 0
                && self.age_max_age_secs >= 0
                && self.access_duration_secs >= 0
                && self.recheck_interval_secs >= 0
                && self.min_age <= 150,
            AccessError::InvalidPolicy
        );
        require!(
            self.min_balance == 0 || self.balance_max_age_secs > 0,
            AccessError::BalanceFreshnessRequired
        );
        Ok(())
    }
}

#[account]
#[derive(InitSpace)]
pub struct AccessPolicy {
    pub authority: Pubkey,
    pub policy_id: u64,
    /// Immutable terms: use a new policy ID when changing terms.
    pub config: AccessPolicyConfig,
    pub paused: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace, Default)]
pub struct AccessGrant {
    pub policy: Pubkey,
    pub user: Pubkey,
    pub first_admitted_at: i64,
    pub last_qualified_at: i64,
    pub expires_at: i64,
    pub next_check_at: i64,
    pub balance_attested_at: i64,
    pub age_attested_at: i64,
    pub balance_source: [u8; 32],
    pub age_source: [u8; 32],
    pub verifier_epoch: u64,
    pub revoked: bool,
    pub bump: u8,
}

pub fn policy_address(authority: &Pubkey, policy_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"access_policy",
            authority.as_ref(),
            &policy_id.to_le_bytes(),
        ],
        &crate::ID,
    )
    .0
}

pub fn grant_address(policy: &Pubkey, user: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"access_grant", policy.as_ref(), user.as_ref()],
        &crate::ID,
    )
    .0
}

fn deadline(now: i64, duration: i64) -> Result<i64> {
    if duration == 0 {
        return Ok(0);
    }
    now.checked_add(duration)
        .ok_or_else(|| error!(AccessError::TimeOverflow))
}

#[derive(Clone, Copy, Default)]
struct Observation {
    at: i64,
    source: [u8; 32],
}

#[derive(Clone, Copy, Default)]
struct AdmissionEvidence {
    balance: Observation,
    age: Observation,
}

fn evidence(
    record: Option<&Account<'_, AttestationRecord>>,
    state: &Account<'_, VerifierState>,
    user: &Pubkey,
    req: gate::Requirement,
) -> Result<Observation> {
    let record = record.ok_or_else(|| error!(AccessError::MissingEvidence))?;
    gate::require_attested_current(record, state, user, &req)?;
    Ok(Observation {
        at: record.attested_at,
        source: record.bank_key_hash,
    })
}

fn qualify(ctx: &QualifyAccess) -> Result<AdmissionEvidence> {
    require!(!ctx.policy.paused, AccessError::PolicyPaused);
    let c = &ctx.policy.config;
    let user = ctx.user.key();
    let mut balance = Observation::default();
    let mut age = Observation::default();
    if c.min_balance > 0 {
        let mut req = gate::Requirement::at_least("balance", c.min_balance)
            .fresh_within(c.balance_max_age_secs);
        req.bank_key_hash = c.balance_source;
        balance = evidence(
            ctx.balance_attestation.as_ref(),
            &ctx.verifier_state,
            &user,
            req,
        )?;
    }
    if c.min_age > 0 {
        let mut req = gate::Requirement::min_age(c.min_age as u64, Clock::get()?.unix_timestamp)
            .fresh_within(c.age_max_age_secs);
        req.bank_key_hash = c.age_source;
        age = evidence(
            ctx.age_attestation.as_ref(),
            &ctx.verifier_state,
            &user,
            req,
        )?;
    }
    Ok(AdmissionEvidence { balance, age })
}

/// Pure transition used by admission and refresh; time is injected for boundary tests.
fn record_qualification(
    grant: &mut AccessGrant,
    policy_and_user: (Pubkey, Pubkey),
    config: &AccessPolicyConfig,
    now: i64,
    evidence: AdmissionEvidence,
    epoch_and_bump: (u64, u8),
    refresh: bool,
) -> Result<()> {
    let (policy, user) = policy_and_user;
    require!(!grant.revoked, AccessError::GrantRevoked);
    let existing = grant.policy != Pubkey::default();
    if existing {
        require!(
            grant.policy == policy && grant.user == user,
            AccessError::WrongGrant
        );
        if refresh {
            require!(
                grant.expires_at == 0 || now < grant.expires_at,
                AccessError::GrantExpired
            );
        } else {
            require!(
                grant.expires_at != 0 && now >= grant.expires_at,
                AccessError::AlreadyActive
            );
        }
        // A mutable balance (or explicitly time-limited DOB) cannot renew access
        // by replaying the old observation. An unbounded stable DOB may be reused.
        require!(
            config.min_balance == 0 || evidence.balance.at > grant.balance_attested_at,
            AccessError::NewEvidenceRequired
        );
        require!(
            config.min_age == 0
                || config.age_max_age_secs == 0
                || evidence.age.at > grant.age_attested_at,
            AccessError::NewEvidenceRequired
        );
    } else {
        require!(!refresh, AccessError::WrongGrant);
    }
    let expiry = if refresh {
        grant.expires_at
    } else {
        deadline(now, config.access_duration_secs)?
    };
    let next = deadline(now, config.recheck_interval_secs)?;
    if !existing {
        grant.first_admitted_at = now;
    }
    grant.policy = policy;
    grant.user = user;
    grant.last_qualified_at = now;
    grant.expires_at = expiry;
    grant.next_check_at = next;
    grant.balance_attested_at = evidence.balance.at;
    grant.balance_source = evidence.balance.source;
    grant.age_attested_at = evidence.age.at;
    grant.age_source = evidence.age.source;
    grant.verifier_epoch = epoch_and_bump.0;
    grant.bump = epoch_and_bump.1;
    Ok(())
}

/// Check the grant without rechecking the age of its admission evidence.
/// Security revocation (issuer removal / verifier epoch) still applies.
pub fn check_access(
    policy: &AccessPolicy,
    grant: &AccessGrant,
    state: &VerifierState,
    expected_policy: &Pubkey,
    user: &Pubkey,
    now: i64,
) -> Result<()> {
    require!(!policy.paused, AccessError::PolicyPaused);
    require!(
        grant.policy == *expected_policy && grant.user == *user,
        AccessError::WrongGrant
    );
    require!(!grant.revoked, AccessError::GrantRevoked);
    require!(
        grant.expires_at == 0 || now < grant.expires_at,
        AccessError::GrantExpired
    );
    require!(
        grant.next_check_at == 0 || now < grant.next_check_at,
        AccessError::RecheckRequired
    );
    require!(
        grant.verifier_epoch == state.policy_epoch,
        AccessError::VerifierRevoked
    );
    for (required, source) in [
        (policy.config.min_balance > 0, grant.balance_source),
        (policy.config.min_age > 0, grant.age_source),
    ] {
        require!(
            !required || state.bank_key_whitelist.contains(&source),
            AccessError::SourceRevoked
        );
    }
    Ok(())
}

/// Consumer integration: pin `expected_policy` from BUSINESS-controlled state,
/// not a user-selected account. Account<T> also checks the verifier program owner.
pub fn require_access_grant(
    policy: &Account<'_, AccessPolicy>,
    grant: &Account<'_, AccessGrant>,
    state: &Account<'_, VerifierState>,
    expected_policy: &Pubkey,
    user: &Pubkey,
) -> Result<()> {
    require_keys_eq!(policy.key(), *expected_policy, AccessError::WrongPolicy);
    require_keys_eq!(
        policy.key(),
        policy_address(&policy.authority, policy.policy_id),
        AccessError::WrongPolicy
    );
    require_keys_eq!(
        grant.key(),
        grant_address(expected_policy, user),
        AccessError::WrongGrant
    );
    let expected_state = Pubkey::find_program_address(&[b"verifier_state"], &crate::ID).0;
    require_keys_eq!(state.key(), expected_state, AccessError::WrongState);
    check_access(
        policy,
        grant,
        state,
        expected_policy,
        user,
        Clock::get()?.unix_timestamp,
    )
}

pub fn create_policy(
    ctx: Context<CreateAccessPolicy>,
    policy_id: u64,
    config: AccessPolicyConfig,
) -> Result<()> {
    config.validate()?;
    let p = &mut ctx.accounts.policy;
    p.authority = ctx.accounts.authority.key();
    p.policy_id = policy_id;
    p.config = config;
    p.paused = false;
    p.bump = ctx.bumps.policy;
    Ok(())
}

pub fn qualify_access(ctx: Context<QualifyAccess>, refresh: bool) -> Result<()> {
    let observations = qualify(ctx.accounts)?;
    record_qualification(
        &mut ctx.accounts.grant,
        (ctx.accounts.policy.key(), ctx.accounts.user.key()),
        &ctx.accounts.policy.config,
        Clock::get()?.unix_timestamp,
        observations,
        (ctx.accounts.verifier_state.policy_epoch, ctx.bumps.grant),
        refresh,
    )
}

#[derive(Accounts)]
#[instruction(policy_id: u64)]
pub struct CreateAccessPolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + AccessPolicy::INIT_SPACE,
        seeds = [b"access_policy", authority.key().as_ref(), &policy_id.to_le_bytes()], bump)]
    pub policy: Account<'info, AccessPolicy>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct QualifyAccess<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [b"access_policy", policy.authority.as_ref(), &policy.policy_id.to_le_bytes()], bump = policy.bump)]
    pub policy: Account<'info, AccessPolicy>,
    #[account(init_if_needed, payer = user, space = 8 + AccessGrant::INIT_SPACE,
        seeds = [b"access_grant", policy.key().as_ref(), user.key().as_ref()], bump)]
    pub grant: Account<'info, AccessGrant>,
    pub balance_attestation: Option<Account<'info, AttestationRecord>>,
    pub age_attestation: Option<Account<'info, AttestationRecord>>,
    #[account(seeds = [b"verifier_state"], bump = verifier_state.bump)]
    pub verifier_state: Box<Account<'info, VerifierState>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ManageAccessPolicy<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority,
        seeds = [b"access_policy", authority.key().as_ref(), &policy.policy_id.to_le_bytes()], bump = policy.bump)]
    pub policy: Account<'info, AccessPolicy>,
}

#[derive(Accounts)]
pub struct ManageAccessGrant<'info> {
    pub authority: Signer<'info>,
    #[account(has_one = authority,
        seeds = [b"access_policy", authority.key().as_ref(), &policy.policy_id.to_le_bytes()], bump = policy.bump)]
    pub policy: Account<'info, AccessPolicy>,
    #[account(mut, has_one = policy,
        seeds = [b"access_grant", policy.key().as_ref(), grant.user.as_ref()], bump = grant.bump)]
    pub grant: Account<'info, AccessGrant>,
}

#[error_code(offset = 7000)]
pub enum AccessError {
    #[msg("Invalid access policy duration, freshness or age")]
    InvalidPolicy,
    #[msg("Balance admission requires a positive freshness limit")]
    BalanceFreshnessRequired,
    #[msg("A required admission attestation is missing")]
    MissingEvidence,
    #[msg("This business policy is paused")]
    PolicyPaused,
    #[msg("Access grant is revoked")]
    GrantRevoked,
    #[msg("Access grant has expired")]
    GrantExpired,
    #[msg("Periodic qualification is due")]
    RecheckRequired,
    #[msg("Wrong business policy")]
    WrongPolicy,
    #[msg("Wrong access grant or user")]
    WrongGrant,
    #[msg("Wrong verifier state")]
    WrongState,
    #[msg("Access is already active; refresh does not extend its expiry")]
    AlreadyActive,
    #[msg("A newer source observation is required for renewal or refresh")]
    NewEvidenceRequired,
    #[msg("Access deadline overflow")]
    TimeOverflow,
    #[msg("Admission evidence was revoked by the verifier")]
    VerifierRevoked,
    #[msg("Admission source was revoked")]
    SourceRevoked,
}

#[cfg(test)]
mod tests;
