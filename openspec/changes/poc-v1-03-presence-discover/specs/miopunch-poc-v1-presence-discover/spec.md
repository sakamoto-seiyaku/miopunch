# miopunch-poc-v1-presence-discover Specification

## Purpose
定义当前 POC v1 的 presence-only Discover：通过 MQTT retained + LWT 形成成员快照，不引入额外目录查询协议。

## ADDED Requirements

### Requirement: Discover uses presence subscription only
The system SHALL implement current POC v1 Discover by subscribing to `mp/v1/net/<net_root>/presence/+`.

The system SHALL use presence only for current online/offline observation. It SHALL NOT require a second directory-query message kind or topology-specific lookup protocol for the default v1 discover path.

#### Scenario: Discover snapshot is built from presence topics
- **WHEN** a current v1 client enters Discover
- **THEN** it subscribes to `mp/v1/net/<net_root>/presence/+`
- **AND** it builds current online/offline evidence from those retained and live presence messages only

### Requirement: Discover view merges presence with the persisted roster
The system SHALL construct the current v1 Discover view by joining presence observations keyed by `peer_id` with the persisted `roster_snapshot` stored by current v1 persistence.

The persisted roster SHALL remain the trust source for member identity, control-plane public keys, and inbox addressing.

#### Scenario: Online state is merged with trusted member identity
- **WHEN** a current v1 client renders Discover for a joined network
- **THEN** it combines presence-derived online/offline state with the persisted roster entry for the same `peer_id`
- **AND** it does not trust the presence payload alone as the source of recipient identity

### Requirement: Presence uses retained online plus retained LWT offline
The system SHALL publish retained `online` on successful broker connect and SHALL configure retained LWT `offline` on the same presence topic.

#### Scenario: Presence reflects connect and unexpected disconnect
- **WHEN** a current v1 peer connects to the broker
- **THEN** it publishes retained `online`
- **AND** if the session drops unexpectedly, retained LWT updates the same topic to `offline`

### Requirement: Presence payload has one fixed JSON field set
The system SHALL encode current v1 presence payload as UTF-8 JSON with this fixed field set:

- `v`
- `state`
- `peer_id`
- `device_name`
- `platform`
- `app_ver`
- `ts_unix_ms`

#### Scenario: Presence payload exposes online state and display hints
- **WHEN** a consumer receives a current v1 presence payload
- **THEN** it can read the peer's current online state and non-secret display hints from the fixed JSON fields
- **AND** it still relies on the persisted roster for trusted control-plane identity

### Requirement: Presence is observation-only
The system SHALL treat current v1 presence as convenience and observability data only.

The system SHALL NOT treat presence as a trust anchor, recipient X25519 source, mailbox authority, or enrollment proof.

#### Scenario: Presence does not grant trust
- **WHEN** a current v1 component consumes a presence payload
- **THEN** it may use the payload for discovery or display
- **AND** it must not use that payload alone as authorization or enrollment proof
