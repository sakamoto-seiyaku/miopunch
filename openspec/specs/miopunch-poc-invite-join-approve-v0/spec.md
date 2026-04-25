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

#### Scenario: Join uses brokers from the invite code
- **WHEN** a joiner receives an invite code with `invite_brokers`
- **THEN** it uses only those broker endpoints for the invite/join/approve exchange
- **AND** each endpoint is in `host:port` form

### Requirement: invite_topic and reply_topic are high entropy and non-enumerable
`invite_topic` and `reply_topic` SHALL be generated as high entropy topic names (≥128 bits effective entropy) and MUST NOT include `peer_id` or user-visible names in plaintext.

#### Scenario: Generated topics do not reveal identity
- **WHEN** the system generates `invite_topic` and `reply_topic`
- **THEN** each topic has at least 128 bits of effective entropy
- **AND** neither topic includes `peer_id` or user-visible names in plaintext

### Requirement: join_request is encrypted with invite_secret
Join requests published to `invite_topic` SHALL be AEAD-encrypted using `invite_secret` (POC v0 uses AES-256-GCM with domain separation).

#### Scenario: Approver decrypts join request with invite secret
- **WHEN** a joiner publishes a `join_request` to `invite_topic`
- **THEN** the request payload is AEAD-encrypted with `invite_secret`
- **AND** the approver can decrypt it using the same invite secret

### Requirement: approve applies uses/idempotency persistently
An issuer/admin node SHALL apply invite/approve idempotency and persistent uses accounting as defined by:

- `miopunch-poc-control-plane-invite-approve-idempotency`

#### Scenario: Replayed join request does not consume another use
- **WHEN** an issuer/admin node receives a duplicate join request for an already handled `request_msg_id`
- **THEN** it reuses the cached response according to invite/approve idempotency
- **AND** it does not decrement `uses_left` a second time

### Requirement: membership_bundle is end-to-end encrypted to joiner
`membership_bundle` delivered to `reply_topic` SHALL be end-to-end encrypted for the joiner.

POC v0 uses X25519 (static ECDH) + HKDF + AES-256-GCM.

#### Scenario: Only the joiner can open the membership bundle
- **WHEN** an approver sends `membership_bundle` to `reply_topic`
- **THEN** the bundle is encrypted for the joiner using X25519, HKDF, and AES-256-GCM
- **AND** a party without the joiner's private key cannot decrypt the bundle
