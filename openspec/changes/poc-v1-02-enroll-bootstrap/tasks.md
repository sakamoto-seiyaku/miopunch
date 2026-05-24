## 1. Enrollment Objects

- [ ] 1.1 Add `internal/pocv1/enroll` and implement `InviteCapability` / `InviteCode (MPINV1)` with the fixed v1 field set, one broker endpoint, and authority signature.
- [ ] 1.2 Implement `JoinRequest` with requester long-term keys, `reply_topic`, optional device metadata, and requester proof-of-possession.
- [ ] 1.3 Implement `MemberCredential` Hard-Min encode/verify logic without storing `peer_id` inside the credential, and define the `roster_snapshot` entry model.

## 2. Authority + Joiner Flow

- [ ] 2.1 Implement the authority-side approve/enroll flow using `01` peer-targeted wire/security primitives only.
- [ ] 2.2 Implement `msg_id`-based authority dedupe and cached response behavior; do not introduce a second request-id system.
- [ ] 2.3 Implement `EnrollResponse` delivery of `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot` only.

## 3. Persistence Handoff + Acceptance

- [ ] 3.1 Persist bootstrap success through `06` APIs only: `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`.
- [ ] 3.2 Add tests for InviteCode roundtrip, JoinRequest PoP verification, MemberCredential verification, and authority restart idempotency.
- [ ] 3.3 Add a local MQTT smoke proving `invite -> join_request -> approve -> enroll_response -> persist` completes and seeds the trusted member roster without touching presence, punch, or session logic.
