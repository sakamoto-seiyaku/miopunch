# miopunch-mqtt-signaling Specification

## Purpose
TBD - created by archiving change add-mqtt-signaling. Update Purpose after archive.
## Requirements
### Requirement: Select MQTT Signaling For Public-Network Experiments
The system SHALL support running NAT traversal experiments without a dedicated `miopunch coord` server by using an MQTT broker as the signaling exchange channel.

#### Scenario: Peers run with MQTT signaling enabled
- **WHEN** both peers are configured to use `mqtt` signaling
- **THEN** they perform signaling via MQTT without connecting to `miopunch coord`
- **AND** the traversal flow still executes `gather -> exchange -> attempt -> dataplane`

### Requirement: YAML Config File For Peer Commands
The system SHALL support a `--config <yaml>` file to provide defaults for peer runs, reducing long command lines.
CLI flags SHALL override values from the config file.

#### Scenario: CLI overrides config values
- **WHEN** `--config` provides `data-proto=quic`
- **AND** the CLI explicitly sets `--data-proto=kcp`
- **THEN** the run uses `kcp`

### Requirement: Session Key Derivation Without Extra User Input
When using MQTT signaling, the system SHALL derive the session identifier from existing inputs:
`proxy name` and `secret`.
The system SHALL NOT require an additional `--session` flag in `P3.5`.

#### Scenario: Both peers derive the same session identifier
- **WHEN** both peers use the same `proxy` and `secret`
- **THEN** they derive the same MQTT session identifier
- **AND** they exchange signaling messages within the same session namespace

### Requirement: Single Broker Is Sufficient For P3.5
For `P3.5`, the system SHALL support configuring exactly one MQTT broker endpoint.
The system SHALL fail fast if the broker is not reachable.

#### Scenario: Run fails when broker cannot be reached
- **WHEN** the broker endpoint is unreachable
- **THEN** signaling fails with stage-level diagnostics

### Requirement: Sync Barrier Before Attempt
When using MQTT signaling, the system SHALL provide a synchronization barrier so both peers start the `attempt` step only after both sides have completed the signaling exchange.

#### Scenario: Visitor starts earlier and waits
- **WHEN** visitor starts first
- **THEN** visitor waits for client presence via the barrier
- **AND** attempt does not start until client is ready

### Requirement: MQTT signaling consumes decision phase plans without backend timing hacks
MQTT signaling SHALL exchange snapshots and align the attempt start window, then pass the resulting `NatHoleResp` data to the backend-neutral attempt executor.

MQTT-specific code SHALL NOT add sender sleeps, receiver timing, or NAT role branches outside the decision/executor phase plan.

#### Scenario: Role timing is not implemented in MQTT session code
- **WHEN** the decision selects sender/receiver behavior for a round
- **THEN** MQTT signaling does not implement role-specific sleeps
- **AND** attempt execution applies the phase plan consistently with other backends

### Requirement: Exchange Uses The Same Program-Defined Information
MQTT signaling SHALL exchange the same program-defined information that the system already uses for traversal decisions:
`direct_addrs`, `mapped_addrs`, `assisted_addrs`,
`tcp_direct_addrs`, `tcp_assisted_addrs`, `tcp_mapped_addrs`, `tcp_stun_cn`, `tcp_stun_global`,
`capabilities`, `p2p_network`,
and selected transport options.

When `p2p_network=tcp_only`, the system SHALL fail fast during exchange if the peer capability set does not include the required TCP Door-2 capability (e.g., `tcp_p2p_v0`).

The decision logic for punching behavior SHALL remain consistent with the existing implementation (same gather snapshot, same neutral punching decision boundary, no trickle updates).

#### Scenario: Exchange results in a usable NatHoleResp snapshot
- **WHEN** both peers complete gather and exchange via MQTT
- **THEN** each peer obtains a `NatHoleResp`-equivalent snapshot for attempt
- **AND** the snapshot includes the exchanged TCP fields when present
- **AND** the attempt step uses this snapshot to establish a usable path (UDP or TCP) consistent with `p2p_network`

### Requirement: NAT Lab Regression Entry Point For MQTT Signaling
The system SHALL provide a repeatable regression entry point in the `P0` NAT lab that runs a representative case using MQTT signaling.

#### Scenario: Run core-01 using MQTT signaling in the lab
- **WHEN** the lab regression suite runs `core-01` with MQTT signaling enabled
- **THEN** the run succeeds
- **AND** logs contain `transport.payload_exchanged` evidence

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

