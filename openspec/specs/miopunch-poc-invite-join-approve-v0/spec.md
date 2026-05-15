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

### Requirement: Approve tasks support explicit review mode
The system SHALL allow an `approve` task to run in explicit review mode.

When an explicit-review approve task receives and validates a `join_request`, it SHALL persist a pending approval request instead of immediately delivering a `membership_bundle`.

The pending approval request SHALL include at minimum: `approve_task_id`, `invite_id`, `request_msg_id`, `member_peer_id`, `status`, `created_at`, and any non-secret joiner display hints that were included in the signed request.

While a request is pending, the system SHALL NOT decrement `uses_left` and SHALL NOT publish a `membership_bundle` for that `request_msg_id`.

Duplicate `join_request` messages with the same `request_msg_id` SHALL update or reuse the same pending approval request instead of creating another pending request.

The system SHALL persist private, validated decision material needed to resolve and publish decisions after daemon restart while the invite is still unexpired. This material SHALL include the request reply topic, validated join request body, member public keys, and invite brokers, and SHALL NOT be exposed through desktop runtime state, LocalAPI responses, task reports, or SSE events.

#### Scenario: Join request waits for explicit approval
- **WHEN** an explicit-review `approve` task receives a valid `join_request`
- **THEN** the system records one pending approval request for that `request_msg_id`
- **AND** no membership bundle is published until an approval decision is accepted
- **AND** invite uses are not decremented while the request is pending

#### Scenario: Duplicate pending request is coalesced
- **WHEN** an explicit-review `approve` task receives the same valid `join_request` more than once
- **THEN** the system exposes one pending approval request for that `request_msg_id`
- **AND** duplicate delivery does not consume another invite use

#### Scenario: Pending request survives daemon restart
- **GIVEN** an explicit-review `approve` task has recorded a pending approval request
- **WHEN** the daemon restarts before the request is decided
- **THEN** the pending request remains visible as an approval request
- **AND** the request remains decision-addressable by `approve_task_id` and `request_msg_id`

### Requirement: Approval decisions are task-addressed and idempotent
The system SHALL support an approval decision task that targets a pending request by `approve_task_id` and `request_msg_id`.

The decision task SHALL accept exactly one decision value: `approve` or `reject`.

The decision task SHALL resolve persisted pending requests without requiring the original `approve` task runtime to still be active.

When the decision is `approve`, the system SHALL apply existing invite idempotency and uses accounting, publish the encrypted `membership_bundle`, cache the response for duplicate `request_msg_id` handling, and mark the approval request `approved`.

When the decision is `reject`, the system SHALL persist a terminal rejection, publish an encrypted rejection response with no membership bundle, and mark the approval request `rejected` without decrementing `uses_left`.

After a request reaches a terminal decision, repeating the same decision SHALL return the existing terminal result without changing invite uses. A conflicting later decision SHALL fail without changing the prior terminal decision.

#### Scenario: Approve decision delivers membership
- **GIVEN** an explicit-review approve task has a pending approval request
- **WHEN** an approval decision task accepts that request
- **THEN** the system publishes the membership bundle to the request reply topic
- **AND** decrements invite uses at most once for that `request_msg_id`
- **AND** the approval request becomes `approved`

#### Scenario: Approve decision after restart delivers membership
- **GIVEN** an explicit-review approve task recorded a pending approval request before daemon restart
- **WHEN** an approval decision task accepts that request after restart
- **THEN** the system publishes the membership bundle to the persisted request reply topic
- **AND** decrements invite uses at most once for that `request_msg_id`
- **AND** the approval request becomes `approved`

#### Scenario: Reject decision denies membership without consuming uses
- **GIVEN** an explicit-review approve task has a pending approval request
- **WHEN** an approval decision task rejects that request
- **THEN** the system publishes a terminal rejection response without a membership bundle
- **AND** invite uses are not decremented for that rejection
- **AND** the approval request becomes `rejected`

#### Scenario: Reject decision after restart denies membership
- **GIVEN** an explicit-review approve task recorded a pending approval request before daemon restart
- **WHEN** an approval decision task rejects that request after restart
- **THEN** the system publishes a terminal rejection response to the persisted request reply topic
- **AND** invite uses are not decremented for that rejection
- **AND** the approval request becomes `rejected`

#### Scenario: Conflicting terminal decision is ignored
- **GIVEN** an approval request has already been approved or rejected
- **WHEN** another decision task submits the opposite decision for the same `request_msg_id`
- **THEN** the task fails without changing the prior decision
- **AND** invite uses remain unchanged

### Requirement: Invite and approve require local admin capability
An `invite`, `approve`, or `approve_decision` task SHALL verify that the current
local identity is an owner or admin in the local governance head before it emits
an invite code, listens for join requests, records approval requests, or
publishes membership material.

The task SHALL fail locally with `FORBIDDEN` when the current identity lacks
admin capability. It SHALL include diagnostic facts identifying the self peer ID
and local governance state.

#### Scenario: Non-admin invite fails before emitting a code
- **GIVEN** a local network exists and the current identity is not owner/admin
- **WHEN** the user starts an invite task
- **THEN** the task fails with `FORBIDDEN`
- **AND** no `invite_code` fact is emitted

#### Scenario: Non-admin approve fails before publishing membership
- **GIVEN** a local network exists and the current identity is not owner/admin
- **WHEN** the user starts approve or approval-decision handling
- **THEN** the task fails with `FORBIDDEN`
- **AND** no approval declaration is added for the joiner

### Requirement: Auto invite mode is rejected until implemented
The system SHALL reject `invite --mode auto` until a complete auto-approval
design is implemented.

The task SHALL fail with `NOT_IMPLEMENTED`, SHALL NOT emit an invite code, and
SHALL suggest using approve mode.

#### Scenario: Auto invite mode is not accepted
- **WHEN** the user starts `invite` with `mode=auto`
- **THEN** the task fails with `NOT_IMPLEMENTED`
- **AND** no `invite_code` fact is emitted
- **AND** diagnostics suggest `--mode approve`

### Requirement: invite preserves reachable hostname broker endpoints
The system SHALL preserve selected reachable hostname broker endpoints in the emitted invite code.

The invite task SHALL still normalize, validate, de-duplicate, and probe
selected endpoints for reachability before emitting the invite code.

#### Scenario: Reachable hostname broker is emitted as hostname
- **WHEN** an invite broker candidate is provided as `host:port`
- **AND** the endpoint passes broker reachability probing
- **THEN** the invite code contains the same normalized `host:port` endpoint
- **AND** the invite code does not replace the hostname with a resolved IP
  address

#### Scenario: Unreachable hostname broker is not emitted
- **WHEN** an invite broker candidate is provided as `host:port`
- **AND** the endpoint fails broker reachability probing
- **THEN** the invite task does not emit that endpoint in `invite_brokers`
- **AND** task diagnostics identify the skipped broker endpoint

### Requirement: Invite broker emission uses hostname-preserving broker selection
The invite task SHALL use a hostname-preserving broker selection path when
emitting `invite_brokers` in invite codes.

The invite task SHALL NOT use a helper that resolves reachable hostname broker
endpoints to A-record IP addresses for invite-code output.

#### Scenario: Reachable hostname broker is not IP-canonicalized for invite code output
- **WHEN** an invite broker candidate is provided as a reachable `host:port`
- **THEN** the emitted invite code contains the selected normalized `host:port`
- **AND** the emitted invite code does not replace that hostname with a resolved
  IP address

#### Scenario: IP-canonicalizing helper is not used by invite emission
- **WHEN** the invite task selects reachable broker endpoints for invite-code
  output
- **THEN** the implementation path preserves the selected endpoint string after
  validation and reachability probing
- **AND** helpers that resolve hostnames to IP addresses are not used for the
  emitted `invite_brokers` value
