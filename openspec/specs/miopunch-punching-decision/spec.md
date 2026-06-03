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

### Requirement: Phase plans drive punching execution
The punching decision boundary SHALL produce phase-plan information sufficient for the attempt executor to run receive-first, bounded punching.

The phase plan SHALL include role, send delay, probe budget, targets, and diagnostic labels. The attempt executor SHALL start the receive side before sending probes and SHALL cancel remaining probes after success.

#### Scenario: UDP receiver opens before sender probes expire
- **GIVEN** a NAT pair that requires receiver-side opening before sender packets can pass
- **WHEN** the peers execute the decision-derived phase plan
- **THEN** the receiver has an active receive loop before the sender's delayed/bounded probes are exhausted
- **AND** success or timeout is recorded with phase diagnostics

### Requirement: MQTT task path uses daemon-lifetime success-only analyzer memory
The MQTT/task product path SHALL use a daemon-lifetime analyzer for punching mode/index recommendations.

Analyzer memory SHALL be scoped by local peer view, remote peer, and protocol, SHALL have bounded lifetime, and SHALL record only successful mode/index recommendation state.

When a signaling exchange computes decision outputs on one side and sends those outputs to the other side, each side SHALL still report success into its own local analyzer scope.

A peer SHALL NOT report success using analyzer metadata that is scoped only to the other peer's local view.

#### Scenario: Repeated peer punching can reuse successful mode preference
- **WHEN** a peer pair succeeds with a punching mode/index
- **THEN** later rounds for the same local peer, remote peer, and protocol can prefer that mode/index
- **AND** each later round still gathers and exchanges fresh candidate snapshots

#### Scenario: Initiator records success in initiator-local scope
- **WHEN** an initiator receives decision output from a responder-led exchange
- **AND** the initiator succeeds with UDP punching
- **THEN** it records success in the initiator daemon's local remote-peer/protocol analyzer scope
- **AND** it does not write success into the responder daemon's analyzer scope

#### Scenario: Responder records success in responder-local scope
- **WHEN** a responder computes decision output and succeeds with UDP punching
- **THEN** it records success in the responder daemon's local remote-peer/protocol analyzer scope
- **AND** the initiator's success memory remains independent

### Requirement: Analyzer metadata is reproducible from exchanged UDP decision material
The punching decision boundary SHALL provide enough metadata or deterministic recomputation inputs for each peer to derive its local analyzer success key from the exchanged UDP decision material.

This metadata SHALL NOT require re-running STUN discovery or changing the selected path after a punch attempt succeeds.

#### Scenario: Local analyzer key can be derived after success
- **WHEN** a peer completes UDP punching using a decision-derived `NatHoleResp`
- **THEN** it can derive the local analyzer key needed to report the successful mode/index
- **AND** derivation uses already exchanged decision material rather than fresh network probing

### Requirement: Phase diagnostics identify receive and probe lifecycle
The system SHALL emit diagnostics that identify receive loop start, probe loop start, first probe, first valid message or connection, winner, timeout, and cancellation reason.

#### Scenario: Timeout includes phase lifecycle evidence
- **WHEN** a punching attempt times out
- **THEN** diagnostics include whether receive and probe loops started
- **AND** diagnostics include the budget and final cancellation or timeout reason

### Requirement: Decision diagnostics do not imply a product coord dependency
Product-facing requirements and diagnostics for MQTT and future signaling backends SHALL describe punching decisions as coming from the decision boundary or decision engine, not from a dedicated `coord` service.

Lab-only logs and commands MAY continue to use `coord` or `coordinator` when they refer specifically to `miopunch-lab coord`.

#### Scenario: Product signaling docs avoid coord as the default mental model
- **WHEN** product or POC documentation describes MQTT or future backend exchange
- **THEN** punching behavior is attributed to a neutral decision boundary
- **AND** `coord` wording is reserved for lab coord service usage or historical notes
