# miopunch-poc-v1-enroll-bootstrap Specification

## Purpose
定义当前 POC v1 的最小 trust bootstrap：`InviteCapability (MPINV1)`、`JoinRequest`、approve/enroll、`MemberCredential` 与 `EnrollResponse`。

## ADDED Requirements

### Requirement: InviteCapability is entry-ticket only
The system SHALL encode `InviteCapability` as `MPINV1-<base64url(no-pad)>` whose payload is TLV bytes.

The payload SHALL carry only the bootstrap fields needed to reach the authority:

- `network_id_bytes`
- `authority_ed25519_pub`
- `authority_x25519_pub`
- `broker`
- `join_topic`
- `invite_id`
- `not_after_unix_ms`
- `sig`

`InviteCapability` MAY carry raw `network_id_bytes` because it is the bootstrap wire artifact.
Joined-state persistence and later cross-capability handoff SHALL use the canonical `network_id` string instead of externalizing a second raw-byte form.

The current extracted v1 invite SHALL carry exactly one broker endpoint for bootstrap delivery.
The system SHALL NOT embed seed peers, topology, mesh state, or long-term transport secrets in the invite.

#### Scenario: InviteCapability carries only bootstrap fields
- **WHEN** a current v1 invite is generated
- **THEN** it contains only the fixed bootstrap fields
- **AND** it does not carry topology or runtime state

### Requirement: JoinRequest submits long-term identity and reply mailbox only
The system SHALL define `JoinRequest` as a peer-targeted request carrying:

- `invite_id`
- `requester_ed25519_pub`
- `requester_x25519_pub`
- `reply_topic`
- optional `device_name`
- optional `platform`
- `created_at_unix_ms`
- `expires_at_unix_ms`
- requester proof-of-possession signature

The system SHALL NOT add a second bootstrap-specific request id beyond the outer/inner `msg_id`.

#### Scenario: Joiner pre-subscribes reply_topic before publishing
- **WHEN** a joiner sends a current v1 `JoinRequest`
- **THEN** it has already subscribed its random `reply_topic`
- **AND** the request body carries only identity and reply mailbox data

### Requirement: JoinRequest is sealed to the authority using current v1 wire/security
The system SHALL publish `JoinRequest` to `join_topic` and SHALL encrypt it to `authority_x25519_pub` using `miopunch-poc-v1-controlplane-wire`.

#### Scenario: Authority is the only intended reader of JoinRequest
- **WHEN** a joiner publishes `JoinRequest`
- **THEN** the message is sealed to `authority_x25519_pub`
- **AND** relays or brokers can route it without reading the request body

### Requirement: MemberCredential binds keys, not peer_id storage
The system SHALL define `MemberCredential` so that membership is bound to:

- `network_id`
- `subject_ed25519_pub`
- `subject_x25519_pub`
- `role`
- `not_before`
- `not_after`
- `issuer_key_id`
- `sig`

`MemberCredential.network_id` SHALL be the canonical external network identifier: the unpadded uppercase base32 encoding of the same raw 16-byte value carried as `network_id_bytes` in the invite.
The system SHALL derive `peer_id` from `subject_ed25519_pub` and SHALL NOT store a second authoritative `peer_id` field inside the credential.

#### Scenario: Credential identity is derived from the signing key
- **WHEN** a current v1 `MemberCredential` is verified
- **THEN** the implementation derives `peer_id` from `subject_ed25519_pub`
- **AND** it rejects the credential if network or signature verification fails

### Requirement: EnrollResponse delivers the minimal enrollment package
After approval, the system SHALL publish `EnrollResponse` to the joiner-provided `reply_topic` using the current v1 wire/security contract.

The encrypted response body SHALL contain only:

- `self_member_credential`
- `mailbox_secret`
- `runtime_broker`
- `roster_snapshot`

Consumers SHALL treat `self_member_credential.network_id` as the authoritative joined-network key for persistence and later handoff, and SHALL NOT key joined state by serializing invite `network_id_bytes` directly.

#### Scenario: Approval returns only bootstrap material
- **WHEN** authority approval succeeds
- **THEN** the joiner receives `EnrollResponse` on `reply_topic`
- **AND** the response body contains only `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`

### Requirement: Enrollment handoff to persistence is atomic
The system SHALL hand `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot` to current v1 persistence as one grouped bootstrap write for the joined network.

Joiners SHALL NOT expose joined-network state after persisting only a strict subset of that four-field package.

#### Scenario: Crash during persistence does not expose a partially joined network
- **WHEN** a joiner crashes or fails while persisting the accepted enrollment package
- **THEN** later startup either sees the complete joined bootstrap state or no joined bootstrap state
- **AND** it does not treat a subset of the four persisted objects as a successful joined network

### Requirement: Initial trusted member roster is bootstrapped during enrollment
The system SHALL treat `roster_snapshot` from `EnrollResponse` as the initial trusted member roster for current v1.

Each roster entry SHALL include:

- `peer_id`
- `MemberCredential`
- optional `device_name`
- optional `platform`

Current v1 consumers MAY use presence to determine online state, but SHALL NOT derive trusted peer identity, recipient X25519 keys, or inbox addressing from presence alone.

#### Scenario: Joiner receives the initial trusted roster at enroll time
- **WHEN** a joiner accepts a valid current v1 `EnrollResponse`
- **THEN** it persists `roster_snapshot` as its initial trusted member roster
- **AND** later discover, punch, and GUI flows can resolve member identity without issuing another directory query capability

### Requirement: Authority idempotency is keyed by message identity
After `miopunch-poc-v1-controlplane-wire` admits a `JoinRequest` at the wire and security boundary, this capability SHALL own authority-side deduplication of the approve/enroll side effect by current v1 message identity (`msg_id`) and SHALL be able to replay a cached response for the same handled request.

#### Scenario: Replayed JoinRequest does not duplicate enrollment
- **WHEN** the authority receives the same handled current v1 `JoinRequest` again
- **THEN** it does not create a second enrollment side effect
- **AND** it may return the cached enrollment response for that `msg_id`
