# Admission policies and independent access grants

A business can require balance, age, both, or neither at admission, then grant
access for a fixed period or indefinitely. Source freshness is checked at admission
and renewal; it is not the access lifetime. A grant records historical qualification,
not continuous possession of money. Business applications must describe this clearly.

## Configuration

`create_access_policy(policy_id, config)` creates immutable terms, owned by the
business authority. Different IDs support multiple products or policy versions.

| Config field | Meaning |
|---|---|
| `min_balance` | Required balance lower bound; zero disables the condition |
| `min_age` | Required completed years (0..150); zero disables the condition |
| `balance_max_age_secs` | Maximum age of the source observation at qualification; positive when balance is required |
| `age_max_age_secs` | Independent DOB evidence age; zero allows reuse of stable DOB evidence |
| `access_duration_secs` | Lifetime from successful admission/renewal; zero means no scheduled expiry |
| `recheck_interval_secs` | Optional periodic qualification deadline; zero means admission/renewal only |
| `balance_source`, `age_source` | Optional trusted issuer key hashes; otherwise any currently whitelisted issuer |

Example Anchor JS config: balance >= 5,000 in the issuer's units, age >=18,
balance observation at most one day old, access for 30 days without periodic checks:

```ts
const config = {
  minBalance: new BN(5000), minAge: 18,
  balanceMaxAgeSecs: new BN(86400), ageMaxAgeSecs: new BN(0),
  accessDurationSecs: new BN(30 * 86400), recheckIntervalSecs: new BN(0),
  balanceSource: null, ageSource: null,
};
await verifier.methods.createAccessPolicy(new BN(1), config)
  .accountsPartial({ authority: businessWallet, policy }).rpc();
```

Use `accessDurationSecs: new BN(0)` for indefinite access. The issuer/bridge policy
defines balance currency and scaling; the numeric proof does not independently
encode a currency. Consumers must select an appropriate issuer/deployment and units.
Changing policy terms requires a new policy ID and explicit consumer adoption;
existing users' terms do not silently change.

## On-chain lifecycle

1. `verify_and_attest` verifies the ZK proof and writes the existing attestation.
2. `admit_access` checks the requested conditions with independent freshness limits,
   correct wallet/claim/PDA, threshold bounds, trusted source and verifier epoch.
   It creates `AccessGrant`, with separate source timestamps and access deadlines.
3. Subsequent use calls `access::require_access_grant`. It requires no source
   attestation accounts, bank request or new ZK proof.
4. At `now >= expires_at` access ends. Calling `admit_access` again performs a
   renewal: it checks admission conditions again and sets a new access period.
5. If periodic checks are configured, access stops at `now >= next_check_at`.
   `refresh_access` rechecks conditions and moves that deadline, **without extending
   the original access expiry**. Expired grants must use admission/renewal instead.

An active grant cannot be readmitted to slide its expiry. Renewal/refresh requires
a strictly newer source observation for balance and for time-limited DOB evidence;
replaying the same proof with a later submission time does not qualify. An unbounded
stable DOB may be reused if it still proves the required age. This avoids unnecessary
identity checks. Nullifier ownership remains in the original verifier record;
expiry/renewal does not free it or implement a second airdrop claim.

Only the business authority can pause its policy or revoke/unrevoke one grant.
Users cannot close grants or bypass revocation by readmission. Unrevoking does not
extend expiry or postpone a periodic check. Removal of an admission issuer from the
verifier whitelist or a verifier epoch bump also invalidates grants, even indefinite
ones. The business can unpause/unrevoke its own decisions; verifier security incidents
require valid current evidence and requalification. Neither kind of refresh extends
access beyond its original expiry unless a full expired-grant renewal succeeds.

## Consumer integration and policy pinning

Store the expected policy address in business-controlled state. Never accept an
arbitrary policy supplied by a user: a different issuer can create a zero-condition
policy. The helper checks the pinned policy, grant/wallet binding, canonical PDAs,
account owners, expiry, periodic deadline, pause and security revocation.

```rust
access::require_access_grant(
    &ctx.accounts.policy, &ctx.accounts.grant, &ctx.accounts.verifier_state,
    &ctx.accounts.business.expected_policy, &ctx.accounts.user.key(),
)?;
```

The consumer must also require the user's signature. This checks control of the
wallet at each action; it does not repeat bank/identity verification. Access grants
do not establish one-person-one-account or replace an application's claim-once rules.

Addresses in the verifier program:

```text
policy = PDA["access_policy", business_authority, policy_id (u64 LE)]
grant  = PDA["access_grant", policy, user_wallet]
```

## Working example and compatibility

`gated-pool` adds `create_access_pool(pool_id)`, `deposit_access(amount)` and
`withdraw_access(amount)`. An `AccessPool` pins the policy at creation. Deposits
consume a grant only. Users may withdraw their own funds after grant expiry,
revocation or policy pause. Account discriminators prevent legacy `deposit` from
bypassing the new access path.

Existing `Pool`, `Position`, `AttestationRecord` and verifier state layouts and
instructions are preserved. Legacy pools continue their existing per-action evidence
checks; new business integrations opt into `AccessPool`/access grants. This is an
on-chain API and tested integration example, not a new business dashboard. Upgrading
deployed program binaries is required before clients can use the new instructions.
No circuit, zkey or proof-format change is required.

## Validation

Validated against commit `564b9a4095041ea9caedd49a86275da34cbedd41`:
[CI](https://github.com/intopic/1/actions/runs/42096538591) and
[ZK E2E](https://github.com/intopic/1/actions/runs/42096538552) both passed, including
the Docker proof test. Localnet passed 30 tests (9 new access cases); the verifier
passed 22 native tests (9 new access cases). See the
[structured validation record](measurements/access-policy-ci.json). No public-network
deployment was performed.

Native tests inject time to exercise exact 30-day expiry and indefinite lifetime,
periodic checks, non-sliding refresh, newer-evidence renewal, invalid configuration,
wallet/policy binding and revocation. Localnet tests use real signed transactions,
fresh DEV proofs and actual validator time, with short test lifetimes rather than
test-only clock or attestation mutation endpoints. The separate full-Circom workflow
continues to validate production proof verification. GitHub Actions uploads localnet
logs and generated IDLs as `access-localnet`.
