# miopunch-poc-control-plane-invite-approve-idempotency Specification

## Purpose
`miopunch-poc-control-plane-invite-approve-idempotency` defines issuer-side idempotency and persistent `uses` accounting for invite/approve flows in POC v0.

## Requirements

### Requirement: Issuer persists invite accounting and handled request responses
For POC v0, an issuer/admin node SHALL persist invite state for each locally issued invite.
The persisted state SHALL be stored at `invites/<invite_id>.json` under the node state directory.

The persisted state SHALL include:
- `invite_topic` (string)
- `expires_at_unix_ms` (int, unix milliseconds)
- `max_uses` (int)
- `uses_left` (int)
- `handled_requests` (object mapping `request_msg_id` to `response_ct_b64url`)

`response_ct_b64url` SHALL be base64url (no-pad) encoding of the response ciphertext bytes.

#### Scenario: Issuer persists handled request response ciphertext
- **WHEN** an issuer successfully handles an invite/approve RPC request
- **THEN** it persists an entry mapping that `request_msg_id` to a cached response ciphertext

### Requirement: uses_left is decremented at most once per request_msg_id
When handling invite/approve RPC requests associated with an invite, the issuer SHALL decrement `uses_left` at most once per unique `request_msg_id`.
If a request with an already-handled `request_msg_id` is received again, the issuer SHALL NOT decrement `uses_left` again.

#### Scenario: Duplicate request does not decrement uses_left again
- **GIVEN** an invite has `uses_left=1`
- **AND** the issuer handles a request with `request_msg_id=X`
- **WHEN** the issuer receives the same request again with `request_msg_id=X`
- **THEN** `uses_left` remains unchanged

### Requirement: Duplicate handled requests cause the cached response to be re-sent
If `handled_requests` contains an entry for a `request_msg_id`, the issuer SHALL respond by re-sending the cached response ciphertext associated with that `request_msg_id`.

#### Scenario: Cached response is re-sent for duplicate request
- **GIVEN** the issuer has a cached response for `request_msg_id=X`
- **WHEN** it receives a duplicate request with `request_msg_id=X`
- **THEN** it re-sends the cached response

### Requirement: Issuer restart does not reset uses_left or handled_requests
The issuer SHALL load `invites/<invite_id>.json` on startup (or before handling invite/approve requests) and apply it as the source of truth for `uses_left` and `handled_requests`.

#### Scenario: Issuer restart preserves uses_left and handled_requests
- **GIVEN** the issuer persisted `uses_left` and a cached response for `request_msg_id=X`
- **WHEN** the issuer restarts and receives the same request again
- **THEN** it does not decrement `uses_left` again
- **AND** it re-sends the cached response

### Requirement: invite_id is deterministically derived from invite_topic
For POC v0, `invite_id` SHALL be derived as `base32(raw,no-pad, sha256(invite_topic)[:16])`.

#### Scenario: Same invite_topic yields same invite_id
- **WHEN** an issuer derives `invite_id` from the same `invite_topic` value
- **THEN** it produces the same `invite_id`
