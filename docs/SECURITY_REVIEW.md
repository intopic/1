# Review of the supplied vulnerability report

The report mixes current issues, integration limitations and findings against the
removed RSA design. Severity is assessed against the current implementation rather
than assumed from the report. Tests are run in the normal CI and ZK E2E workflows.

## VULN-01: prefunded PDA initialization

The reported permanent one-lamport DoS does not match Anchor's initialization code.
Anchor 0.31.1 and the locked 0.31.3 implementation use transfer/allocate/assign when
a system-owned destination already has lamports. Replacing `init_if_needed` is not
necessary to address this scenario and would remove legitimate refresh behavior.

Localnet regressions send funds from a separate wallet before initializing verifier
state, attestation, nullifier, legacy position, business policy, access pool, grant
and access position. A one-lamport transfer is attempted; if runtime rent rules
reject or reclaim it, the attacker funds the empty-account rent minimum instead.
Transfers are confirmed before checking the persistent account. Subsequent
normal initialization and use must succeed. Grant revocation and nullifier ownership
remain enforced after initialization; the code exposes no user-controlled close path.

Source: [Anchor 0.31.1 account initialization](https://github.com/coral-xyz/anchor/blob/v0.31.1/lang/syn/src/codegen/accounts/constraints.rs).

## VULN-02: proxy IP trust

The old arbitrary header option accepted the first comma-separated value and could
be unsafe behind a proxy that appends client-controlled X-Forwarded-For. Direct
deployment defaults used the socket peer; the documented Fly deployment used
Fly-Client-IP. This was a conditional configuration vulnerability, not an automatic
bypass of every deployment.

The option now accepts only `fly-client-ip` or `cf-connecting-ip`. The selected
header must contain one valid IP; duplicate, missing or list-valued headers on
protected POST routes fail with 400. Unselected headers are ignored. Native parsing
tests and real loopback HTTP tests verify the rate-limit bucket cannot be changed
with forged X-Forwarded-For values in direct mode or alongside a selected header.

**Network trust remains required:** these names do not authenticate a sender.
Only enable the option when every untrusted request passes through the named proxy,
which overwrites that header, and direct origin access is prevented. Cloudflare in
front of Fly needs its own explicitly designed ingress policy; this code does not
guess trust from a forwarded chain. Limits remain per daemon, not fleet-wide.

Source: [Fly request headers and spoofing cautions](https://docs.fly.io/networking/request-headers).

## VULN-03: bank HTTP compatibility and wallet binding

GET-only and the custom bank header were integration restrictions. The claim that
all bank WAFs reject unknown headers is not established. Browser CORS is not enforced
by this native Rust TCP/HTTP client. Nevertheless, avoiding custom bank headers is a
useful compatibility improvement.

The wallet now lives in the `intopic.zktls.wallet/v1` TLSNotary attestation extension.
Its exact 32-byte value is signed by the notary along with the attestation. The
notary accepts exactly this one extension, the prover validates the returned
attestation, and the bridge extracts it only after signature/presentation validation.
Duplicate/missing/malformed extensions and a different requested wallet are rejected.
The bank receives no `x-zktls-wallet` header. Wallet ownership is still enforced by
the signer/public-input check when the proof is submitted to Solana.

POST support is deliberately limited to **policy-approved read-only queries**:
the source specifies `method: "POST"` and an exact public JSON `request_body` string.
The entire body and HTTP framing must be authenticated, with one matching
Content-Length, JSON Content-Type, matching Host and no Transfer-Encoding or pipelined
request. The method is part of source matching. Dynamic or secret POST bodies need
a separate policy design; do not place credentials in a body that is fully disclosed
to the bridge. Authentication header values remain hidden from the presentation.

```json
{
  "method": "POST",
  "request_body": "{\"query\":\"current\"}",
  "server": "api.example.test",
  "path_prefix": "/v1/accounts/me",
  "target_key": "balance",
  "account_key": "iban",
  "currency": "EUR"
}
```

Use `zktls-fetch --method POST --body-file query.json --header 'Content-Type: application/json' ...`.
The file must match the configured bytes, including whitespace. GET remains the
default for existing policy JSON. Read-only endpoint selection is the operator's
responsibility; transfer/payment endpoints must not be enabled as data sources.

**Migration:** upgrade notary, fetch client and bridge together and regenerate old
TLSNotary presentations. Header-only presentations are rejected rather than silently
accepted without a signed binding. The v3 canonical message, circuit and zkey remain
unchanged. Existing on-chain attestations/grants are not invalidated by this transport
change. Gzip/Brotli/chunked responses are still unsupported; `Accept-Encoding: identity`
is sent but cannot force every server to comply. No universal bank compatibility is
claimed; real providers require their own authorization and compatibility tests.

Source: the pinned [TLSNotary alpha.15 attestation extensions](https://github.com/tlsnotary/tlsn/tree/v0.1.0-alpha.15/crates/attestation/src).

## VULN-04: canonical point encoding

Explicit Fq range checks now cover every G1/G2 coordinate in A, B, C, the verifying
key and its IC points before arithmetic. Negating a zero y-coordinate preserves zero
instead of producing the noncanonical modulus. Tests mutate every proof coordinate
and verifying-key component and also verify rejection of a canonical off-curve point.
The native syscalls still perform curve and subgroup validation. The old implementation
already delegated malformed points to those syscalls; the report does not demonstrate
an accepted forged proof. This is input-validation hardening and a negation edge-case fix.

## Architecture recommendations

An additional CI issue was found during reproduction: piping `anchor test` through
`tee` without `pipefail` could mark a job successful despite Mocha failures. The
localnet step now enables `set -euo pipefail`, and a Node runner validates a fresh
Mocha JSON report with no failures/skips and all three required suites. Regression
tests deliberately supply failing, empty, skipped and incomplete reports. Both
human-readable logs and the JSON report are uploaded as artifacts.

* **RSA in the ZK circuit:** already removed before this report. Current measured v3
  has 6,582 constraints and a roughly 4.12 MiB zkey, not 277,321 constraints. Phone
  proving in 50 ms and a 2 MB key are not established measurements.
* **Custom bigint/RSA/AVX:** no longer used for attestation signature verification.
  Production prover and bridge builds now enable only triad-core's normalizer.
  Historical arithmetic modules are feature-gated under `arithmetic`; standalone
  library defaults preserve their public API and workspace tests still cover them.
  CI builds both normalizer-only and arithmetic `no_std` configurations. Unused direct
  CLI and workspace RSA dependency declarations were removed.
* **One Poseidon public-input digest:** a possible future circuit/protocol change,
  not a drop-in fix. Solana would have to recompute the same hash over all fields it
  uses for authorization; trusting unbound metadata would break verification. Fewer
  IC points reduce MSM work and VK size, but on-chain hashing adds work. The asserted
  80,000 CU saving is not measured. The existing authority-restricted two-step VK
  upload remains; no circuit/key/ceremony migration is made on that claim alone.

For existing measured performance see [v3](V3.md) and [parallel results](PARALLEL_RESULTS.md).
