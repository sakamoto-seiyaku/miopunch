# miopunch-poc-v1-controlplane-wire Specification

## Purpose
定义当前 POC v1 peer-targeted 控制面消息的唯一事实源：TLV bytes、outer/inner envelope、固定 transcript、`peer_e2e_v1`、drop-only errors 与 golden vectors。

## Requirements

### Requirement: Current v1 peer-targeted messages use binary TLV bytes over MQTT
The system SHALL encode current POC v1 peer-targeted control messages as binary TLV bytes and SHALL transport them as MQTT payload bytes.

The system SHALL NOT use JSON as the signed or encrypted wire source; JSON is only allowed for GUI/log readability.

#### Scenario: The same logical message encodes deterministically
- **WHEN** a current v1 peer-targeted message is encoded twice with the same logical inputs
- **THEN** the emitted TLV bytes are identical
- **AND** a receiver can decode those bytes back to the same logical message

### Requirement: TLV encoding is canonical and strict
The system SHALL encode TLV as `tag(uvarint) || len(uvarint) || value(bytes)`.

The system SHALL reject any current v1 peer-targeted message that contains:

- unknown tags
- duplicate tags
- non-canonical uvarint encodings for `tag` or `len`
- fields not in strictly increasing tag order
- any length/value mismatch or truncation

#### Scenario: Non-canonical or duplicate fields are rejected
- **WHEN** a receiver decodes a TLV message with a duplicate tag or a non-canonical uvarint
- **THEN** it rejects the message as malformed

### Requirement: Outer and inner envelopes define the security boundary
The system SHALL carry each current v1 peer-targeted message as:

- an outer relay header in plaintext TLV
- an encrypted inner peer message in TLV

The system SHALL treat outer `src` as route/debug information only and SHALL derive sender identity only from the decrypted and verified inner message.

#### Scenario: Outer src mismatch does not create trust
- **WHEN** the outer `src` disagrees with the sender identity proven by the inner message
- **THEN** authorization and handling use only the inner identity
- **AND** the outer `src` is not treated as a trusted sender claim

### Requirement: Current v1 kind ownership is split by layer
The system SHALL allow only these top-level current v1 peer-targeted `kind` names:

- `join_request`
- `enroll_response`
- `dial_offer`
- `dial_answer`

This capability SHALL freeze only the top-level `kind` names and envelope carriage rules.
The body schema for `join_request` and `enroll_response` SHALL be defined by `miopunch-poc-v1-enroll-bootstrap`.
The body schema for `dial_offer` and `dial_answer` SHALL be defined by `miopunch-poc-v1-dial-punch`.

#### Scenario: Unsupported kind is dropped before body handling
- **WHEN** a receiver opens a current v1 inner message whose `kind` is outside the four-name allowlist
- **THEN** it drops the message without side effects

### Requirement: Current v1 network identity has one canonical external form
The system SHALL treat `network_id` as the canonical external network identifier across current v1 capabilities.

`network_id` MUST be the unpadded uppercase base32 encoding of one raw 16-byte network value and therefore be exactly 26 ASCII characters.
Any current v1 field named `network_id_bytes` SHALL refer to that same value in raw 16-byte form and SHALL be used only where bytes are required for wire, TLV, or crypto internals rather than the canonical external identifier.

#### Scenario: Canonical network identity round-trips to raw bytes
- **WHEN** a current v1 capability converts a canonical `network_id` to `network_id_bytes` and back
- **THEN** it recovers the same raw 16-byte value
- **AND** the canonical external form remains the 26-character uppercase base32 string

### Requirement: Current v1 signatures use one fixed transcript
The system SHALL define the current v1 signing input as `domain-sep + TLV(fields...)` with one fixed field order.

The system SHALL sign transcript bytes directly with Ed25519 and SHALL NOT use JSON canonicalization or pre-hash as the signature source.

#### Scenario: Transcript bytes are stable across runs
- **WHEN** the transcript is generated for the same logical current v1 inner message
- **THEN** the transcript bytes are identical across runs

### Requirement: Current v1 peer_e2e_v1 is sign-then-encrypt recipient-only
The system SHALL implement `peer_e2e_v1` as sign-then-encrypt, recipient-only E2E for current v1 peer-targeted messages.

The system SHALL derive the AEAD key using X25519 + HKDF-SHA256, encrypt using XChaCha20-Poly1305, and bind AAD only to the trusted outer header fields (`v`, `dst`, `msg_id`, `expires_at`, `scheme`).

#### Scenario: Ciphertext or AAD tampering fails
- **WHEN** any ciphertext byte or trusted AAD byte is modified in transit
- **THEN** the message is rejected and not delivered to the application

### Requirement: Current v1 errors are drop-only
The system SHALL handle decrypt failure, bad signature, expiry, replay, unsupported version, and malformed input by local drop only.

The system SHALL NOT send a network error reply for those failures.
The replay and expiry checks frozen here are wire or security admission hooks only.
This capability SHALL NOT define business-level side-effect deduplication or cached-response policy after a message is successfully opened and admitted.

#### Scenario: Invalid peer-targeted input is dropped locally
- **WHEN** a receiver detects decrypt failure, bad signature, expiry, replay, unsupported version, or malformed bytes
- **THEN** it drops the message locally
- **AND** it updates only local evidence or error aggregation

### Requirement: Golden vectors are mandatory for the extracted v1 wire
The system SHALL provide byte-level golden vectors covering:

- outer header TLV determinism
- inner message TLV determinism
- transcript determinism
- ciphertext frame determinism for fixed eph/nonce fixtures
- tamper cases that fail to open or verify

#### Scenario: Golden vectors match byte-for-byte
- **WHEN** the implementation runs the current v1 wire test suite against published fixtures
- **THEN** it matches the expected bytes exactly for outer, inner, transcript, and ciphertext vectors
