# miopunch-poc-invite-join-approve-v0 Specification

## Purpose
`miopunch-poc-invite-join-approve-v0` defines the POC v0 invitation and join flow:

- `invite` generates an invite code with `invite_topic`, `invite_secret`, `invite_brokers`, and expiry/uses policy
- `join` publishes an encrypted `join_request` and waits for `membership_bundle`
- `approve` listens on `invite_topic`, applies idempotency + uses accounting, and delivers `membership_bundle`

## Requirements

### Requirement: invite code pins broker endpoints
Invite codes SHALL include `invite_brokers` with `1..2` broker endpoints in `host:port` form.

During `invite/join/approve`, the system SHALL use only the endpoints provided by `invite_brokers`.

### Requirement: invite_topic and reply_topic are high entropy and non-enumerable
`invite_topic` and `reply_topic` SHALL be generated as high entropy topic names (≥128 bits effective entropy) and MUST NOT include `peer_id` or user-visible names in plaintext.

### Requirement: join_request is encrypted with invite_secret
Join requests published to `invite_topic` SHALL be AEAD-encrypted using `invite_secret` (POC v0 uses AES-256-GCM with domain separation).

### Requirement: approve applies uses/idempotency persistently
An issuer/admin node SHALL apply invite/approve idempotency and persistent uses accounting as defined by:

- `miopunch-poc-control-plane-invite-approve-idempotency`

### Requirement: membership_bundle is end-to-end encrypted to joiner
`membership_bundle` delivered to `reply_topic` SHALL be end-to-end encrypted for the joiner.

POC v0 uses X25519 (static ECDH) + HKDF + AES-256-GCM.

