# miopunch-poc-v1-presence-discover Specification

## Purpose
Defines POC v1 peer discovery via MQTT presence only (retained + LWT).

## ADDED Requirements

### Requirement: Discover uses presence only
The system SHALL implement Discover by subscribing to `mp/v1/net/<net_root>/presence/+` and SHALL NOT require additional directory query message kinds.

#### Scenario: Discover snapshot comes from presence subscription
- **WHEN** a v1 client enters the Discover stage
- **THEN** it derives the network presence prefix and subscribes to `mp/v1/net/<net_root>/presence/+`
- **AND** it does not issue a separate directory query message

### Requirement: Presence uses retained + LWT
The system SHALL publish retained `online` on successful connect and SHALL configure retained LWT `offline` on the same presence topic.

#### Scenario: Presence reflects online and offline transitions
- **WHEN** a peer connects successfully to the v1 broker
- **THEN** it publishes retained `online` on its presence topic
- **AND** if the session drops unexpectedly, the retained LWT updates that same topic to `offline`

### Requirement: Presence payload includes control-plane public keys
Presence payload SHALL include the peer's Ed25519 and X25519 public keys (as base64url no-pad strings) to support dial_offer encryption.

#### Scenario: Discover snapshot exposes peer control-plane keys
- **WHEN** a v1 client receives a retained or live presence payload
- **THEN** the payload includes the peer's Ed25519 and X25519 public keys
- **AND** the client can use the X25519 key for later `dial_offer` encryption
