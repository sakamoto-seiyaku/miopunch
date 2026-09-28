## 1. Enrollment Objects

- [x] 1.1 Add `internal/pocv1/enroll` and implement `InviteCapability` / `InviteCode (MPINV1)` with the fixed v1 field set, one broker endpoint, and authority signature.
- [x] 1.2 Implement `JoinRequest` with requester long-term keys, `reply_topic`, optional device metadata, and requester proof-of-possession.
- [x] 1.3 Implement `MemberCredential` Hard-Min encode/verify logic without storing `peer_id` inside the credential, and define the `roster_snapshot` entry model.

## 2. Authority + Joiner Flow

- [x] 2.1 Implement the authority-side approve/enroll flow using `01` peer-targeted wire/security primitives only.
- [x] 2.2 Implement `msg_id`-based authority dedupe and cached response behavior; do not introduce a second request-id system.
- [x] 2.3 Implement `EnrollResponse` delivery of `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot` only.

## 3. Persistence Handoff + Acceptance

- [x] 3.1 Persist bootstrap success through one `06` atomic grouped-bootstrap API only: `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`.
- [x] 3.2 Add tests for InviteCode roundtrip, JoinRequest PoP verification, MemberCredential verification, authority restart idempotency, and key negative paths: invite/member-credential tamper rejection, invalid enroll response structure, `msg_id` fingerprint mismatch, and joiner persistence handoff error semantics.
- [x] 3.3 Add local MQTT smoke coverage proving `invite -> join_request -> approve -> enroll_response -> persist` completes and seeds the trusted member roster without touching presence, punch, or session logic, plus a replay-cache smoke showing repeated `JoinRequest` delivery for the same handled `msg_id` returns the cached authority response.
