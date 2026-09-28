## MODIFIED Requirements

### Requirement: Secure sessions borrow the runtime UDP socket
The system SHALL upgrade a POC v1 `PathResult` into KCP, TLS 1.3, and yamux
without taking ownership of Runtime's UDP socket.

Closing a failed transport attempt or a live `PeerSession` SHALL close only the
session-owned resources. It SHALL NOT close the UDP socket borrowed through the
path result.

#### Scenario: Failed TLS/KCP upgrade keeps UDP retryable
- **GIVEN** a session upgrade attempt uses Runtime's UDP socket
- **WHEN** the KCP/TLS/yamux upgrade fails
- **THEN** cleanup closes the session-owned resources
- **AND** the Runtime UDP socket remains usable for another punch/session attempt

#### Scenario: Closing a PeerSession keeps UDP retryable
- **GIVEN** a live `PeerSession` uses Runtime's UDP socket
- **WHEN** Runtime closes that peer session because it is superseded, fatal, or shutting down
- **THEN** the `PeerSession` closes its stream and transport resources
- **AND** the Runtime UDP socket remains usable until Runtime itself closes
