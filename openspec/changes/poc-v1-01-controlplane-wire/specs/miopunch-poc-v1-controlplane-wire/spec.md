# miopunch-poc-v1-controlplane-wire Specification

## Purpose
Defines the POC v1 peer-targeted control-plane wire: TLV encoding, outer/inner envelope, signature transcript rules, and peer_e2e_v1 recipient-only E2E semantics.

## ADDED Requirements

### Requirement: v1 peer-targeted control messages use TLV bytes over MQTT
The system SHALL encode v1 peer-targeted control-plane messages as binary TLV bytes and transport them as-is over MQTT payloads.

The system SHALL NOT use JSON as the signed or encrypted wire source (JSON is only for GUI/log output).

#### Scenario: A v1 control message roundtrips
- **WHEN** a v1 control message is encoded and then decoded
- **THEN** the decoded message equals the original logical message
- **AND** the encoder output is deterministic for the same inputs

### Requirement: v1 kind allowlist is fixed
The system SHALL accept only the following v1 peer-targeted message kinds:

- `join_request`
- `enroll_response`
- `dial_offer`
- `dial_answer`

All other kinds SHALL be rejected and dropped.

#### Scenario: Unsupported kind is dropped
- **WHEN** a receiver decodes a v1 inner message whose `kind` is not in the allowlist
- **THEN** it drops the message without side effects

### Requirement: Outer/inner envelope is fixed
The system SHALL carry v1 peer-targeted messages as an outer relay header (plaintext TLV) wrapping an encrypted inner peer message (TLV).

The system SHALL treat outer `src` as not security-relevant, and SHALL derive sender identity only from the decrypted+verified inner message.

#### Scenario: Receiver ignores outer src for identity
- **WHEN** a receiver receives a v1 outer header whose `src` does not match the sender identity inside the decrypted inner message
- **THEN** it uses only the inner message identity for authorization/handling decisions
- **AND** it does not treat the mismatch as a reason to trust the outer `src`

### Requirement: TLV encoding is canonical and strict
The system SHALL encode TLV as `tag(uvarint) || len(uvarint) || value(bytes)`.

The system SHALL reject any message that contains:

- Unknown tags
- Duplicate tags
- Non-canonical uvarint encodings for `tag` or `len`
- Fields not in strictly increasing tag order
- Any length/value mismatch or truncation

#### Scenario: Duplicate tag is rejected
- **WHEN** a receiver decodes a TLV message that contains the same tag twice
- **THEN** it rejects the message as malformed

#### Scenario: Non-canonical uvarint is rejected
- **WHEN** a receiver decodes a TLV message that uses a non-canonical uvarint encoding for `tag` or `len`
- **THEN** it rejects the message as malformed

### Requirement: v1 signatures use a fixed transcript
The system SHALL define the signature input (transcript) as `domain-sep + TLV(fields...)` with a fixed field order.

The system SHALL sign the transcript bytes directly with Ed25519 (no pre-hash), and SHALL NOT use JSON canonicalization as the signature source.

#### Scenario: Transcript bytes are stable
- **WHEN** the transcript is generated for the same logical message
- **THEN** the transcript bytes are identical across runs

### Requirement: peer_e2e_v1 is sign-then-encrypt recipient-only
The system SHALL provide `peer_e2e_v1` as sign-then-encrypt, recipient-only E2E for peer-targeted control-plane messages.

The system SHALL derive the AEAD key using X25519 + HKDF-SHA256, encrypt using XChaCha20-Poly1305, and bind AAD to the outer header (excluding `ct` and excluding untrusted `src`).

#### Scenario: Tampering breaks verification
- **WHEN** any ciphertext or AAD byte is modified in transit
- **THEN** the message is rejected and not delivered to the application

### Requirement: Golden vectors are required
The system SHALL provide golden vectors (byte-level fixtures, hex) covering:

- Outer header TLV encoding determinism
- Inner message TLV + transcript determinism
- Ciphertext frame determinism when eph_priv and nonce are fixed
- Tamper cases (ciphertext/AAD) fail to decrypt or verify

#### Scenario: Golden vectors validate byte-for-byte
- **WHEN** the implementation runs tests against the published golden vectors
- **THEN** it byte-for-byte matches the expected hex for outer/inner/transcript/ciphertext fixtures
