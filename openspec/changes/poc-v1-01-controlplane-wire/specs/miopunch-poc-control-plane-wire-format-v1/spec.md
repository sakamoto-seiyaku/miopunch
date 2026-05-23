# miopunch-poc-control-plane-wire-format-v1 Specification

## Purpose
Defines the POC v1 control-plane wire encoding, signature transcript rules, and peer-targeted E2E envelope semantics.

## ADDED Requirements

### Requirement: Control-plane peer-targeted messages use v1 TLV wire
The system SHALL encode v1 peer-targeted control-plane messages as binary TLV bytes and transport them as-is over MQTT payloads.

#### Scenario: A v1 control message roundtrips
- **WHEN** a v1 control message is encoded and then decoded
- **THEN** the decoded message equals the original logical message
- **AND** the encoder output is deterministic for the same inputs

### Requirement: v1 signatures use a fixed transcript
The system SHALL define the signature input (transcript) as `domain-sep + TLV(fields...)` with a fixed field order, and SHALL NOT use JSON canonicalization as the signature source.

#### Scenario: Transcript bytes are stable
- **WHEN** the transcript is generated for the same logical message
- **THEN** the transcript bytes are identical across runs

### Requirement: peer_e2e_v1 is sign-then-encrypt recipient-only
The system SHALL provide `peer_e2e_v1` as sign-then-encrypt, recipient-only E2E for peer-targeted control-plane messages.

#### Scenario: Tampering breaks verification
- **WHEN** any ciphertext or AAD byte is modified in transit
- **THEN** the message is rejected and not delivered to the application
