## ADDED Requirements

### Requirement: Punching phase scheduling is backend-neutral
The punching decision boundary SHALL produce attempt behavior that is independent of the signaling backend used to exchange snapshots.

The punching phase plan SHALL include the role, delay, target set, budget, and cancellation semantics needed by the attempt executor. Signaling backends SHALL NOT encode NAT sender/receiver timing as backend-specific sleeps or special cases.

#### Scenario: MQTT exchange does not own NAT role timing
- **WHEN** MQTT signaling has delivered both peer snapshots and a common start window
- **THEN** the punching decision and executor determine receiver/sender timing from the phase plan
- **AND** MQTT-specific code does not add NAT role sleeps

### Requirement: Success-only analyzer memory is scoped per peer and protocol
The system SHALL support a daemon-lifetime, success-only analyzer memory for punching mode/index recommendations.

The memory SHALL be scoped by peer and protocol, SHALL NOT record endpoint/candidate/winning target information, and SHALL NOT record failure reasons.

#### Scenario: Successful punching can influence later mode recommendation
- **WHEN** a peer pair succeeds using a punching mode/index for a protocol
- **THEN** the analyzer can prefer that mode/index in later rounds for the same peer and protocol
- **AND** the later round still gathers and exchanges fresh network snapshots

#### Scenario: Dataplane failure does not write punching failure memory
- **WHEN** punching establishes a path but the dataplane payload exchange fails
- **THEN** the punching analyzer does not record a failure for that mode/index
- **AND** dataplane diagnostics own the payload failure
