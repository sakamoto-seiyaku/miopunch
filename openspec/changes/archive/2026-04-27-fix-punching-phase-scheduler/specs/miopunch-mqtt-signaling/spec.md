## ADDED Requirements

### Requirement: MQTT signaling consumes decision phase plans without backend timing hacks
MQTT signaling SHALL exchange snapshots and align the attempt start window, then pass the resulting `NatHoleResp` data to the backend-neutral attempt executor.

MQTT-specific code SHALL NOT add sender sleeps, receiver timing, or NAT role branches outside the decision/executor phase plan.

#### Scenario: Role timing is not implemented in MQTT session code
- **WHEN** the decision selects sender/receiver behavior for a round
- **THEN** MQTT signaling does not implement role-specific sleeps
- **AND** attempt execution applies the phase plan consistently with other backends
