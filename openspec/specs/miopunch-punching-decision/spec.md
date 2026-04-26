# miopunch-punching-decision Specification

## Purpose
`miopunch-punching-decision` defines the service-neutral punching decision boundary that turns exchanged peer snapshots into attempt-ready `NatHoleResp` outputs without requiring a dedicated coord service.

## Requirements
### Requirement: Punching decision boundary is service-neutral
The system SHALL provide a service-neutral punching decision boundary that derives visitor/client `NatHoleResp` outputs from exchanged `NatHoleVisitor` and `NatHoleClient` snapshots without requiring a running `coord` service.

The decision boundary SHALL be reusable by MQTT-led exchange, future mailbox/overlay signaling, and the lab coord service adapter.

#### Scenario: MQTT leader uses the decision boundary without coord service semantics
- **WHEN** MQTT signaling has exchanged visitor and client NAT-hole snapshots
- **THEN** the MQTT leader can derive visitor and client `NatHoleResp` outputs through the neutral decision boundary
- **AND** the product path does not need to depend on the lab coord service package

#### Scenario: Lab coord remains an adapter over the same decision boundary
- **WHEN** `miopunch-lab coord` receives visitor and client NAT-hole snapshots
- **THEN** it derives the same `NatHoleResp` outputs through the neutral decision boundary
- **AND** the lab coord service remains available for experiments and regression runs

### Requirement: Decision outputs preserve the attempt contract
The punching decision boundary SHALL preserve the existing attempt-ready response contract:
- data plane selection consistency (`Protocol`, `QuicCC`, and brutal limits)
- effective `p2p_network`
- peer direct, assisted, UDP candidate, and TCP candidate addresses
- STUN view selection metadata when available
- UDP punching enablement and detect behavior
- TCP punching enablement, error attribution, and detect behavior

The boundary SHALL continue to use a single exchanged gather snapshot per side and SHALL NOT introduce trickle candidate updates.

#### Scenario: Decision output can be consumed by attempt unchanged
- **WHEN** a signaling backend passes exchanged snapshots to the decision boundary
- **THEN** each peer receives a `NatHoleResp` that `connectivity.Attempt` can consume without a signaling-specific translation step
- **AND** no additional wire fields are required for this cleanup

### Requirement: Decision diagnostics do not imply a product coord dependency
Product-facing requirements and diagnostics for MQTT and future signaling backends SHALL describe punching decisions as coming from the decision boundary or decision engine, not from a dedicated `coord` service.

Lab-only logs and commands MAY continue to use `coord` or `coordinator` when they refer specifically to `miopunch-lab coord`.

#### Scenario: Product signaling docs avoid coord as the default mental model
- **WHEN** product or POC documentation describes MQTT or future backend exchange
- **THEN** punching behavior is attributed to a neutral decision boundary
- **AND** `coord` wording is reserved for lab coord service usage or historical notes
