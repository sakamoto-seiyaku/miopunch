## ADDED Requirements

### Requirement: MQTT signaling buckets attempts by dial_id within a SID
When using MQTT signaling, the system SHALL support concurrent traversal attempts within the same `SID` by bucketing signaling topics by `dial_id`.

`dial_id` SHALL be derived from an existing per-attempt identifier and SHALL be unique within the `SID` scope.

The signaling exchange topics (info/resp/ready/start) for an attempt SHALL be derived from:
`<topicPrefix>/<sid>/attempt/<dial_id>/...`.

#### Scenario: Two visitors do not stomp each other within the same SID
- **WHEN** two visitors start traversal against the same client within the same `SID` concurrently
- **THEN** each visitor observes responses only for its own `dial_id` bucket
- **AND** both attempts can reach a stable `start` barrier without topic-level interference

### Requirement: Visitor publishes info/visitor before client responds for a dial_id
The MQTT signaling flow SHALL bind an attempt to a `dial_id` bucket by requiring the visitor to publish `info/visitor` first.

The client SHALL respond only after observing `info/visitor` for a specific `dial_id`.

#### Scenario: Client binds a dial_id after observing info/visitor
- **WHEN** a visitor publishes `attempt/<dial_id>/info/visitor`
- **THEN** the client processes that attempt within the same `dial_id` bucket
- **AND** the client publishes `attempt/<dial_id>/info/client` and later `resp/*` within that bucket

### Requirement: Barrier and start semantics are per dial_id bucket
The MQTT signaling barrier (`ready/*`) and aligned `start` window SHALL be scoped to a single `dial_id` bucket.

The system SHALL NOT use a single shared `start` topic for all attempts within a `SID`.

#### Scenario: Two attempts observe independent start windows
- **WHEN** two attempts with different `dial_id` values run concurrently within the same `SID`
- **THEN** each attempt observes its own `attempt/<dial_id>/start` message
- **AND** the barrier for one attempt does not unblock or delay the other attempt

