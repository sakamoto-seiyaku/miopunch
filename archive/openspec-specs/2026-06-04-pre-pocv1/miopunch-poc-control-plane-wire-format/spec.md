# miopunch-poc-control-plane-wire-format Specification

## Purpose
`miopunch-poc-control-plane-wire-format` defines the POC v0 control-plane message plaintext structure, signature transcript coverage rules, and the ciphertext framing used when carrying messages over mesh and MQTT mailbox delivery.

## Requirements

### Requirement: Control-plane plaintext is UTF-8 JSON with fixed top-level fields
For POC v0, the system SHALL encode control-plane plaintext messages as UTF-8 JSON with the following fixed top-level fields:
- `proto_version` (int)
- `route` (object)
- `signed` (object)

The `route` object SHALL contain:
- `dst_peer_id` (string)
- `msg_id` (string)
- `hop_limit` (int)
- `created_at_unix_ms` (int)
- `expires_at_unix_ms` (int, optional)

The `signed` object SHALL contain:
- `sender_peer_id` (string)
- `kind` (string, snake_case)
- `in_reply_to` (string, optional)
- `body` (JSON value)
- `sig_b64` (string, base64url no-pad)

#### Scenario: Encode and decode preserves the message structure
- **WHEN** a sender encodes a control-plane message with `proto_version`, `route`, and `signed`
- **THEN** a receiver can decode it and observe the same field values

### Requirement: Signature transcript covers dst_peer_id and excludes hop_limit
The system SHALL compute the signature transcript from the following fields, in this exact order:
`dst_peer_id + msg_id + created_at_unix_ms + expires_at_unix_ms? + sender_peer_id + kind + in_reply_to? + body`

The system SHALL include `route.dst_peer_id` in the transcript.
The system SHALL NOT include `route.hop_limit` in the transcript.

#### Scenario: Changing dst_peer_id invalidates the signature
- **WHEN** a valid signed message is modified by changing `route.dst_peer_id`
- **THEN** signature verification fails

#### Scenario: Changing hop_limit does not invalidate the signature
- **WHEN** a valid signed message is modified by changing `route.hop_limit` only
- **THEN** signature verification still succeeds

### Requirement: Receiver enforces dst_peer_id equals self before acting
After signature verification succeeds, the receiver SHALL check that `route.dst_peer_id` equals the receiver's own `peer_id`.
If `dst_peer_id` does not match `self_peer_id`, the receiver SHALL drop the message without applying any side effects.

#### Scenario: Receiver drops messages not addressed to self
- **WHEN** a receiver verifies a message whose `route.dst_peer_id` is not equal to its own peer_id
- **THEN** the receiver drops the message

### Requirement: Ciphertext framing is v||nonce||ct with AES-256-GCM v=0
When carrying control-plane messages as ciphertext bytes (over mesh or MQTT), the system SHALL use the following framing:
`v(1B) || nonce(12B) || ct`

The system SHALL use `v=0` to indicate AES-256-GCM.
The system SHALL use a 12-byte nonce for AES-256-GCM.

#### Scenario: Ciphertext framing roundtrips for valid inputs
- **WHEN** a sender encrypts a plaintext message using v=0 framing
- **THEN** a receiver can decrypt and recover the original plaintext

#### Scenario: Unsupported ciphertext version is rejected
- **WHEN** a receiver receives a ciphertext frame whose `v` is not supported
- **THEN** it rejects the ciphertext frame

### Requirement: POC v0 control-plane wire is legacy-only after v1 extraction
The system SHALL treat the JSON/AES-GCM control-plane wire defined by `miopunch-poc-control-plane-wire-format` as a legacy POC v0 contract only.

Current POC v1 peer-targeted control-plane messages SHALL use `miopunch-poc-v1-controlplane-wire` as their source of truth and SHALL NOT reuse this legacy capability as the runtime contract for current v1 extraction work.

#### Scenario: Current v1 implementation chooses the v1 wire contract
- **WHEN** a developer implements or reviews the current POC v1 peer-targeted control-plane path
- **THEN** they use `miopunch-poc-v1-controlplane-wire` as the governing contract
- **AND** they treat the JSON/AES-GCM capability as historical reference for archived POC v0 only
