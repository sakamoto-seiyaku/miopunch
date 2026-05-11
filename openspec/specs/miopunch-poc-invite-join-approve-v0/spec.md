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

`invite` SHALL verify that selected `invite_brokers` are reachable before emitting an invite code.

If broker connection fails during `invite/join/approve`, the task SHALL fail with `UNAVAILABLE`, report the broker endpoint that failed, and provide broker reachability/configuration guidance.

#### Scenario: Join uses brokers from the invite code
- **WHEN** a joiner receives an invite code with `invite_brokers`
- **THEN** it uses only those broker endpoints for the invite/join/approve exchange
- **AND** each endpoint is in `host:port` form

#### Scenario: Invite does not emit a code for an unreachable broker
- **WHEN** invite broker verification cannot connect to the selected broker
- **THEN** the invite task fails with `UNAVAILABLE`
- **AND** no `invite_code` fact is emitted
- **AND** task diagnostics identify the broker endpoint and broker reachability/configuration action

#### Scenario: Join and approve report broker endpoint failures
- **WHEN** join or approve cannot connect to an invite broker endpoint
- **THEN** the task fails with `UNAVAILABLE`
- **AND** task diagnostics identify the broker endpoint and broker reachability/configuration action

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

### Requirement: Owner/admin selects current effective brokers from explicit or built-in candidates
The system SHALL derive `brokers_effective` from exactly one candidate source
when an owner/admin node creates or maintains current network broker state:

- if explicit `local.mqtt_broker` configuration exists, use only that
  configured list;
- otherwise, use the built-in broker candidate list.

The system SHALL normalize, de-duplicate, and probe reachability for those
candidates, then keep at most two reachable endpoints in source order as the
current `brokers_effective` pair.

#### Scenario: Explicit local broker configuration wins over built-in defaults
- **WHEN** an owner/admin node has explicit
  `local.mqtt_broker=["broker-a:1883","broker-b:1883","broker-c:1883"]`
- **THEN** it selects `brokers_effective` only from that explicit list
- **AND** it does not mix in built-in broker candidates

#### Scenario: Built-in broker defaults are used only when explicit config is absent
- **WHEN** an owner/admin node has no explicit `local.mqtt_broker` configuration
- **THEN** it derives `brokers_effective` from the built-in broker candidate
  list
- **AND** the resulting effective broker set contains at most two reachable
  endpoints

### Requirement: Membership applies the full effective broker set to persisted peer signaling state
When a membership bundle includes `brokers_effective`, the system SHALL persist
the full effective broker set as the post-join MQTT signaling broker state for
that net.

On successful `join`, the joiner SHALL save its local `mqtt_broker` state from
the membership bundle's `brokers_effective` list before future runtime
signaling.

On successful `approve`, the approver SHALL save the joiner's peer config from
that same full `brokers_effective` list before that config is used for future
`ping` or `sh` signaling.

#### Scenario: Joiner listens on the effective broker set after join
- **WHEN** a joiner receives a valid `membership_bundle` with
  `brokers_effective=["broker-a:1883","broker-b:1883"]`
- **THEN** the joiner's saved local state uses that effective broker set
- **AND** subsequent acceptor signaling uses the same primary/secondary pair

#### Scenario: Approver dials joiner through the effective broker set
- **WHEN** an approver accepts a join request from a joiner whose seed peer
  advertised a different broker before membership
- **THEN** the approver saves the joiner's peer config with the net's effective
  broker set
- **AND** subsequent peer tasks from the approver to that joiner use the same
  primary/secondary broker pair
