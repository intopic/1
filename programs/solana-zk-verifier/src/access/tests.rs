use super::*;
use crate::VerifyingKey;

const DAY: i64 = 86_400;
const NOW: i64 = 1_800_000_000;

fn policy() -> AccessPolicy {
    AccessPolicy {
        authority: Pubkey::new_unique(),
        policy_id: 42,
        paused: false,
        bump: 255,
        config: AccessPolicyConfig {
            min_balance: 5000,
            min_age: 18,
            balance_max_age_secs: DAY,
            age_max_age_secs: 0,
            access_duration_secs: 30 * DAY,
            recheck_interval_secs: 0,
            balance_source: None,
            age_source: None,
        },
    }
}

fn state() -> VerifierState {
    VerifierState {
        authority: Pubkey::new_unique(),
        verifying_key: VerifyingKey {
            alpha_g1: [0; 64],
            beta_g2: [0; 128],
            gamma_g2: [0; 128],
            delta_g2: [0; 128],
            ic: vec![],
        },
        bank_key_whitelist: vec![[7; 32]],
        app_id: [0; 32],
        allowed_claim_keys: vec![gate::BALANCE, gate::DATE_OF_BIRTH],
        max_attestation_age_secs: 3600,
        bump: 255,
        policy_epoch: 3,
        pending_authority: Pubkey::default(),
    }
}

fn observations(at: i64) -> AdmissionEvidence {
    AdmissionEvidence {
        balance: Observation {
            at,
            source: [7; 32],
        },
        age: Observation {
            at: NOW - 365 * DAY,
            source: [7; 32],
        },
    }
}

fn admitted(p: &AccessPolicy) -> (Pubkey, Pubkey, AccessGrant) {
    let key = policy_address(&p.authority, p.policy_id);
    let user = Pubkey::new_unique();
    let mut grant = AccessGrant::default();
    record_qualification(
        &mut grant,
        (key, user),
        &p.config,
        NOW,
        observations(NOW),
        (3, 255),
        false,
    )
    .unwrap();
    (key, user, grant)
}

fn fails<T>(result: Result<T>, name: &str) {
    assert!(
        result.err().unwrap().to_string().contains(name),
        "expected {name}"
    );
}

#[test]
fn access_lifetime_is_independent_of_balance_freshness() {
    let p = policy();
    let (key, user, g) = admitted(&p);
    // Evidence is 29 days old, but the user was admitted for 30 days.
    assert!(check_access(&p, &g, &state(), &key, &user, NOW + 29 * DAY).is_ok());
    assert!(check_access(&p, &g, &state(), &key, &user, NOW + 30 * DAY - 1).is_ok());
    fails(
        check_access(&p, &g, &state(), &key, &user, NOW + 30 * DAY),
        "GrantExpired",
    );
}

#[test]
fn indefinite_access_has_no_scheduled_expiry() {
    let mut p = policy();
    p.config.access_duration_secs = 0;
    let (key, user, g) = admitted(&p);
    assert_eq!(g.expires_at, 0);
    assert!(check_access(&p, &g, &state(), &key, &user, NOW + 20 * 365 * DAY).is_ok());
}

#[test]
fn refresh_requires_new_balance_and_does_not_slide_access_expiry() {
    let mut p = policy();
    p.config.recheck_interval_secs = DAY;
    let (key, user, mut g) = admitted(&p);
    fails(
        check_access(&p, &g, &state(), &key, &user, NOW + DAY),
        "RecheckRequired",
    );
    fails(
        record_qualification(
            &mut g,
            (key, user),
            &p.config,
            NOW + DAY,
            observations(NOW),
            (3, 255),
            true,
        ),
        "NewEvidenceRequired",
    );
    record_qualification(
        &mut g,
        (key, user),
        &p.config,
        NOW + DAY,
        observations(NOW + DAY),
        (3, 255),
        true,
    )
    .unwrap();
    assert_eq!(g.expires_at, NOW + 30 * DAY);
    assert_eq!(g.next_check_at, NOW + 2 * DAY);
    assert!(check_access(&p, &g, &state(), &key, &user, NOW + DAY).is_ok());
}

#[test]
fn renewal_requires_expiry_and_new_observation() {
    let p = policy();
    let (key, user, mut g) = admitted(&p);
    fails(
        record_qualification(
            &mut g,
            (key, user),
            &p.config,
            NOW + 1,
            observations(NOW + 1),
            (3, 255),
            false,
        ),
        "AlreadyActive",
    );
    let end = NOW + 30 * DAY;
    fails(
        record_qualification(
            &mut g,
            (key, user),
            &p.config,
            end,
            observations(NOW),
            (3, 255),
            false,
        ),
        "NewEvidenceRequired",
    );
    record_qualification(
        &mut g,
        (key, user),
        &p.config,
        end,
        observations(end),
        (3, 255),
        false,
    )
    .unwrap();
    assert_eq!(g.first_admitted_at, NOW);
    assert_eq!(g.expires_at, end + 30 * DAY);
}

#[test]
fn revoked_grant_cannot_be_recreated_or_refreshed() {
    let p = policy();
    let (key, user, mut g) = admitted(&p);
    g.revoked = true;
    fails(
        check_access(&p, &g, &state(), &key, &user, NOW),
        "GrantRevoked",
    );
    for refresh in [true, false] {
        fails(
            record_qualification(
                &mut g,
                (key, user),
                &p.config,
                NOW + 31 * DAY,
                observations(NOW + 31 * DAY),
                (3, 255),
                refresh,
            ),
            "GrantRevoked",
        );
    }
}

#[test]
fn business_user_and_security_revocation_are_enforced() {
    let mut p = policy();
    let (key, user, g) = admitted(&p);
    fails(
        check_access(&p, &g, &state(), &key, &Pubkey::new_unique(), NOW),
        "WrongGrant",
    );
    fails(
        check_access(&p, &g, &state(), &Pubkey::new_unique(), &user, NOW),
        "WrongGrant",
    );
    p.paused = true;
    fails(
        check_access(&p, &g, &state(), &key, &user, NOW),
        "PolicyPaused",
    );
    p.paused = false;
    let mut st = state();
    st.policy_epoch += 1;
    fails(
        check_access(&p, &g, &st, &key, &user, NOW),
        "VerifierRevoked",
    );
    st.policy_epoch -= 1;
    st.bank_key_whitelist.clear();
    fails(check_access(&p, &g, &st, &key, &user, NOW), "SourceRevoked");
}

#[test]
fn policies_validate_independent_freshness_and_lifetime() {
    let mut p = policy();
    p.config.validate().unwrap();
    p.config.access_duration_secs = 0;
    p.config.validate().unwrap();
    p.config.balance_max_age_secs = 0;
    fails(p.config.validate(), "BalanceFreshnessRequired");
    p.config.min_balance = 0;
    p.config.validate().unwrap();
    p.config.min_age = 151;
    fails(p.config.validate(), "InvalidPolicy");
    p.config.min_age = 18;
    p.config.recheck_interval_secs = -1;
    fails(p.config.validate(), "InvalidPolicy");
}

#[test]
fn refresh_cannot_revive_expired_access_or_create_a_grant() {
    let p = policy();
    let (key, user, mut g) = admitted(&p);
    fails(
        record_qualification(
            &mut g,
            (key, user),
            &p.config,
            NOW + 30 * DAY,
            observations(NOW + 30 * DAY),
            (3, 255),
            true,
        ),
        "GrantExpired",
    );
    fails(
        record_qualification(
            &mut AccessGrant::default(),
            (key, user),
            &p.config,
            NOW,
            observations(NOW),
            (3, 255),
            true,
        ),
        "WrongGrant",
    );
    fails(deadline(NOW, i64::MAX), "TimeOverflow");
}

#[test]
fn stable_age_can_renew_without_unnecessary_identity_recheck() {
    let mut p = policy();
    p.config.min_balance = 0;
    let (key, user, mut g) = admitted(&p);
    record_qualification(
        &mut g,
        (key, user),
        &p.config,
        NOW + 30 * DAY,
        observations(NOW),
        (3, 255),
        false,
    )
    .unwrap();
    p.config.age_max_age_secs = DAY;
    fails(
        record_qualification(
            &mut g,
            (key, user),
            &p.config,
            NOW + 60 * DAY,
            observations(NOW),
            (3, 255),
            false,
        ),
        "NewEvidenceRequired",
    );
}
