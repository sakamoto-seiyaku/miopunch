# miopunch-poc-control-plane-mesh-first-fallback Specification

## Purpose
`miopunch-poc-control-plane-mesh-first-fallback` defines the POC v0 delivery policy for control-plane request/response messages: mesh-first delivery with MQTT mailbox fallback, and the deduplication behavior required to prevent dual-path delivery from causing duplicate side effects.

## Requirements

### Requirement: Request delivery is mesh-first with MQTT fallback
When a node has at least one mesh neighbor available, it SHALL attempt to deliver control-plane request/response messages via mesh first.
If no valid response is observed within 1 second, it SHALL publish the same ciphertext payload to the destination peer's MQTT inbox topic as a fallback.

#### Scenario: Mesh-first delivery succeeds without requiring MQTT fallback
- **WHEN** a sender has a mesh path to the destination and sends a request
- **THEN** the destination receives the request via mesh and produces a valid response
- **AND** the request is considered successful without requiring MQTT fallback

#### Scenario: MQTT fallback delivers when mesh delivery does not respond in time
- **WHEN** a sender sends a request via mesh but observes no response within 1 second
- **THEN** it publishes the request ciphertext to the destination inbox topic via MQTT
- **AND** the destination can receive and respond via the MQTT mailbox path

### Requirement: Dual-path delivery does not cause duplicate side effects
If a receiver observes the same message via multiple delivery paths (e.g., both mesh and MQTT), it SHALL apply at most one set of side effects for that message ID.

If the message is an RPC request (when `signed.kind` ends with `_request`) and the receiver has already produced a final response for that `request_msg_id`, the receiver SHALL re-send the cached final response on duplicate deliveries.

#### Scenario: Receiver processes duplicate deliveries at most once
- **WHEN** a receiver receives the same `msg_id` via mesh and MQTT within the dedup window
- **THEN** it processes the message at most once

#### Scenario: Receiver re-sends cached final response for duplicate RPC request
- **GIVEN** a receiver already produced a final response for `request_msg_id=X`
- **WHEN** it receives a duplicate RPC request again with `route.msg_id=X`
- **THEN** it re-sends the cached final response

### Requirement: LAN smoke is reproducible with three processes
The system SHALL provide a reproducible LAN smoke harness that can be executed as three separate processes on the same LAN segment:
- node A (sender)
- node B (forwarder)
- node C (receiver)

The harness SHALL demonstrate:
- bounded flooding (H=3) forwarding via B from A to C
- mesh-first request delivery from A to C
- MQTT fallback without duplicate side effects

#### Scenario: Three-process LAN smoke validates mesh-first and MQTT fallback
- **WHEN** the three nodes are started with a neighbor topology A↔B↔C and a reachable MQTT broker
- **THEN** a request from A to C can complete successfully
- **AND** duplicate side effects from dual-path delivery do not occur

### Requirement: Nodes maintain active neighbors after join
After joining a net, a node SHALL maintain active neighbors according to `k=max(2,ceil(ln(n)))`, where `n` is the known legal peer count.

Neighbor selection SHALL use reachability bucket ordering, online state, and random or rotating selection within a bucket. The system SHALL avoid selecting only the best-connected admin nodes when enough alternatives exist.

#### Scenario: Joined node chooses multiple active neighbors
- **WHEN** a node knows 12 legal peers
- **THEN** it targets approximately three active neighbors
- **AND** the selected neighbor set is not limited to the primary admin when other eligible peers exist

### Requirement: Neighbor health drives reconnect or replacement
A node SHALL monitor active neighbor health through data-plane activity, keepalive, or equivalent product-level evidence.

If an active neighbor becomes unhealthy, the node SHALL attempt bounded reconnect. If reconnect fails within budget, the node SHALL select a replacement candidate using the same reachability bucket policy.

For MNT-03, the lab MAY trigger a bounded product neighbor-maintenance cycle, but candidate selection, dialing, health evidence, and failure reporting SHALL be produced by product code and exposed through topology diagnostics.

#### Scenario: Unhealthy neighbor is replaced
- **WHEN** an active neighbor is offline long enough to fail health checks
- **THEN** the node attempts bounded reconnect
- **AND** after reconnect failure it selects a replacement candidate
- **AND** topology diagnostics report the reconnect and replacement evidence

### Requirement: Active neighbor edges carry payload evidence
An active neighbor edge SHALL be considered established only when the system has transport evidence for the edge.

For MNT-03, transport evidence SHALL include selected attempt path, data protocol, peer IDs, and a successful payload exchange or an explainable failure with stop condition.

#### Scenario: Active edge includes transport evidence
- **WHEN** a node reports a peer as an active neighbor
- **THEN** topology diagnostics include attempt path and data protocol evidence for that edge
- **AND** a gate can validate payload success or explainable failure for that edge
