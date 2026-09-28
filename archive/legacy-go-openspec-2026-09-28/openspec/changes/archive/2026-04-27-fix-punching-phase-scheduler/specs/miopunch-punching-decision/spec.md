## ADDED Requirements

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

Analyzer memory SHALL be scoped by peer and protocol, SHALL have bounded lifetime, and SHALL record only successful mode/index recommendation state.

#### Scenario: Repeated peer punching can reuse successful mode preference
- **WHEN** a peer pair succeeds with a punching mode/index
- **THEN** later rounds for the same peer/protocol can prefer that mode/index
- **AND** each later round still gathers and exchanges fresh candidate snapshots

### Requirement: Phase diagnostics identify receive and probe lifecycle
The system SHALL emit diagnostics that identify receive loop start, probe loop start, first probe, first valid message or connection, winner, timeout, and cancellation reason.

#### Scenario: Timeout includes phase lifecycle evidence
- **WHEN** a punching attempt times out
- **THEN** diagnostics include whether receive and probe loops started
- **AND** diagnostics include the budget and final cancellation or timeout reason
