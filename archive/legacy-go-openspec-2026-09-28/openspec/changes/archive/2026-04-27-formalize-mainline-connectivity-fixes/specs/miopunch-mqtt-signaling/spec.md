## ADDED Requirements

### Requirement: MQTT exchange readiness remains separate from punching phase scheduling
MQTT signaling SHALL provide exchange readiness and start-window alignment for NAT-hole rounds, but SHALL NOT implement backend-specific NAT role timing.

The attempt behavior derived from `NatHoleResp` SHALL be executable by a backend-neutral punching phase scheduler.

#### Scenario: MQTT delivers a round without owning phase timing
- **WHEN** both peers complete MQTT exchange for a NAT-hole round
- **THEN** each peer receives an attempt-ready response
- **AND** sender delay, receiver preparation, probe budget, and cancellation are handled by punching decision/executor logic
