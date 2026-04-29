## ADDED Requirements

### Requirement: UDP data plane complies with the socket owner / demux boundary
When the selected connectivity path is `UDP`, the data plane SHALL be compatible with a single UDP socket owner / demux architecture:

- The data plane MUST NOT require direct ownership of UDP packet receiving (`ReadFrom*`) when traversal is also active on the same UDP mapping.
- The data plane implementation SHALL support coexistence with traversal packets on the same UDP socket / port mapping.

#### Scenario: Data plane does not require exclusive UDP receive ownership
- **GIVEN** traversal and data plane share a single UDP socket / port mapping
- **WHEN** the system establishes a UDP-based peer transport session
- **THEN** the session can run without violating the “single socket owner / demux” constraint
- **AND** traversal packets do not corrupt or terminate an established data plane session

### Requirement: UDP acceptor supports multi-peer concurrent session accept
For UDP transports (`kcp` and `quic`), the server / acceptor side data plane SHALL support accepting multiple peer transport sessions over the same UDP port.

The accept loop MUST NOT be single-shot. A long-lived session MUST NOT block other peers from establishing their own sessions to the same acceptor port.

#### Scenario: Existing session does not block new inbound session
- **GIVEN** an acceptor has already accepted and is serving a UDP peer transport session
- **WHEN** another peer attempts to establish a new session to the same acceptor UDP port
- **THEN** the acceptor accepts the new session without requiring the old session to close
- **AND** both sessions can be used to open logical streams

