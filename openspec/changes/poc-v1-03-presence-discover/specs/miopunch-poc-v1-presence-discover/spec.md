# miopunch-poc-v1-presence-discover Specification

## Purpose
Defines POC v1 peer discovery via MQTT presence only (retained + LWT).

## ADDED Requirements

### Requirement: Discover uses presence only
The system SHALL implement Discover by subscribing to `mp/v1/net/<net_root>/presence/+` and SHALL NOT require additional directory query message kinds.

### Requirement: Presence uses retained + LWT
The system SHALL publish retained `online` on successful connect and SHALL configure retained LWT `offline` on the same presence topic.

### Requirement: Presence payload includes control-plane public keys
Presence payload SHALL include the peer's Ed25519 and X25519 public keys (as base64url no-pad strings) to support dial_offer encryption.
