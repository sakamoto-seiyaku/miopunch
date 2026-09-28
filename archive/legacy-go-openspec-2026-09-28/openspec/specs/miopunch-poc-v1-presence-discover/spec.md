# miopunch-poc-v1-presence-discover Specification

## Purpose
定义当前 POC v1 的 presence-only Discover：通过 MQTT retained + LWT 形成成员快照，不引入额外目录查询协议。

## Requirements

### Requirement: Discover uses presence subscription only
The system SHALL implement current POC v1 Discover by subscribing to `mp/v1/net/<net_root>/presence/+`.

The system SHALL use presence only for current online/offline observation. It SHALL NOT require a second directory-query message kind or topology-specific lookup protocol for the default v1 discover path.

#### Scenario: Discover snapshot is built from presence topics
- **WHEN** a current v1 client enters Discover
- **THEN** it subscribes to `mp/v1/net/<net_root>/presence/+`
- **AND** it builds current online/offline evidence from those retained and live presence messages only

### Requirement: Discover view has one fixed domain contract
The system SHALL expose one current v1 domain discover contract named `DiscoverView`.

`DiscoverView` SHALL contain only:

- `network_id`
- `self_peer_id`
- `observed_at_unix_ms`
- `peers[]`

Each `DiscoverPeer` entry in `peers[]` SHALL contain only:

- `peer_id`
- `online_state`
- `device_name`
- `platform`
- optional `app_ver`
- optional `last_observed_unix_ms`

`DiscoverView.peers[]` SHALL contain at most one entry per trusted remote `peer_id` and SHALL exclude `self_peer_id`.

The current v1 discover contract SHALL NOT carry `MemberCredential`, remote `x25519`, inbox topics, or dial/session state inside `DiscoverView`.

#### Scenario: Trusted remote peers appear in one fixed shape
- **WHEN** a current v1 runtime materializes Discover for one joined network
- **THEN** it emits one `DiscoverView`
- **AND** each trusted remote peer appears at most once in `DiscoverView.peers[]`
- **AND** the view excludes the local `self_peer_id`

### Requirement: Discover view is bounded by the persisted roster
The system SHALL construct the current v1 `DiscoverView` by joining presence observations keyed by `peer_id` with the persisted `roster_snapshot` stored by current v1 persistence.

The persisted roster SHALL remain the trust source for member identity, control-plane public keys, and inbox addressing.

The persisted roster SHALL also define the discover peer-set boundary for the default current v1 path.

Presence-only observations for `peer_id` values that do not exist in the persisted roster SHALL stay diagnostic-only and SHALL NOT enter `DiscoverView.peers[]`.

#### Scenario: Online state is merged with trusted member identity
- **WHEN** a current v1 client renders Discover for a joined network
- **THEN** it combines presence-derived online/offline state with the persisted roster entry for the same `peer_id`
- **AND** it does not trust the presence payload alone as the source of recipient identity

#### Scenario: Trusted remote peer defaults to offline without presence
- **WHEN** a trusted remote peer exists in `roster_snapshot`
- **AND** no retained or live presence message has been observed for that `peer_id`
- **THEN** the peer still appears in `DiscoverView.peers[]`
- **AND** its `online_state` is `offline`

#### Scenario: Unknown presence-only peer stays out of DiscoverView
- **WHEN** a current v1 consumer receives a presence observation for a `peer_id` that is absent from `roster_snapshot`
- **THEN** that observation does not create a `DiscoverPeer`
- **AND** the runtime may surface only diagnostic/evidence output for that unknown peer

### Requirement: Presence lifecycle converges through retained online and offline
The system SHALL publish retained `online` on successful broker connect and SHALL configure retained LWT `offline` on the same presence topic.

The system SHALL also publish retained `offline` on graceful shutdown before disconnecting from the broker.

#### Scenario: Presence reflects connect and unexpected disconnect
- **WHEN** a current v1 peer connects to the broker
- **THEN** it publishes retained `online`
- **AND** if the session drops unexpectedly, retained LWT updates the same topic to `offline`

#### Scenario: Graceful shutdown converges to offline on the same topic
- **WHEN** a current v1 peer shuts down cleanly
- **THEN** it publishes retained `offline` to its presence topic
- **AND** later retained snapshot readers observe that peer as `offline`

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

### Requirement: Presence display hints merge conservatively
The system SHALL merge presence display hints into `DiscoverView` conservatively.

For `device_name` and `platform`, the persisted roster SHALL remain canonical when it already carries a non-empty value.

Presence-derived `device_name` or `platform` MAY backfill a blank roster field, but SHALL NOT overwrite a non-empty roster field.

`app_ver` MAY be carried from presence because it is observation-only and is not part of roster trust authority.

#### Scenario: Roster display hints win over conflicting presence hints
- **WHEN** a trusted roster entry already has non-empty `device_name` or `platform`
- **AND** a presence payload for the same `peer_id` carries a different value
- **THEN** `DiscoverView` keeps the roster value for that field
- **AND** presence may still contribute optional `app_ver` and observation time

### Requirement: Invalid presence observations do not mutate DiscoverView
Malformed JSON, unsupported `v`, invalid `peer_id`, or a topic/payload `peer_id` mismatch SHALL NOT mutate the current v1 `DiscoverView`.

These observations MAY produce typed diagnostics or evidence, but SHALL remain outside the trusted discover view.

#### Scenario: Invalid observation is ignored for DiscoverView state
- **WHEN** a current v1 consumer receives a malformed or invalid presence payload
- **THEN** the runtime does not add or modify any `DiscoverPeer`
- **AND** it may record diagnostic evidence for operators or GUI consumers

### Requirement: Presence is observation-only
The system SHALL treat current v1 presence as convenience and observability data only.

The system SHALL NOT treat presence as a trust anchor, recipient X25519 source, mailbox authority, or enrollment proof.

#### Scenario: Presence does not grant trust
- **WHEN** a current v1 component consumes a presence payload
- **THEN** it may use the payload for discovery or display
- **AND** it must not use that payload alone as authorization or enrollment proof

### Requirement: Last-seen object model is presence-owned and minimal
If the current v1 runtime materializes last-seen presence state, it SHALL use this minimal `LastSeenPeer` model:

- `peer_id`
- `last_state`
- `last_observed_unix_ms`
- optional `last_online_unix_ms`

The current v1 presence capability SHALL freeze that object model before any later persistence wiring, and current v1 persistence foundation SHALL NOT be required to invent additional `last_seen_peers` schema for this change.

#### Scenario: Last-seen shape is fixed before persistence wiring
- **WHEN** a current v1 implementer prepares to persist last-seen presence data
- **THEN** the runtime already has the minimal `LastSeenPeer` model defined by this capability
- **AND** the implementer does not expand current v1 persistence foundation first just to discover the object shape
