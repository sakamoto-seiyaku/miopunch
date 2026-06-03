## ADDED Requirements

### Requirement: POC v1 Runtime-owned UDP paths use one owner for traversal and KCP
For current POC v1 Runtime-owned UDP paths, the system SHALL use one Runtime-owned UDP owner/demux boundary for both traversal and KCP.

Only the Runtime-owned UDP owner SHALL receive packets from the underlying Runtime UDP socket.

The Runtime-owned UDP owner SHALL expose:

- a traversal demux view for direct handshake and UDP punching packets
- a KCP packet transport view for non-traversal KCP packets

POC v1 punch and secure-session code SHALL NOT create independent raw `ReadFromUDP` loops over the Runtime UDP socket.

#### Scenario: Runtime UDP owner routes traversal and KCP packets
- **GIVEN** current POC v1 Runtime owns a UDP socket
- **WHEN** a tag-prefixed traversal packet arrives
- **THEN** the Runtime UDP owner routes it to traversal demux handling
- **AND** KCP does not receive that packet

#### Scenario: KCP packets use the owner PacketConn
- **GIVEN** current POC v1 Runtime owns a UDP socket
- **WHEN** KCP session establishment starts for a Runtime-owned selected path
- **THEN** KCP reads and writes through the Runtime owner PacketConn view
- **AND** it does not call `ReadFromUDP` on the raw Runtime UDP socket

#### Scenario: No competing Runtime UDP readers
- **WHEN** current POC v1 accepts an inbound punch and dials an outbound punch concurrently
- **THEN** both traversal attempts use the Runtime owner traversal demux
- **AND** no per-attempt raw UDP traversal demux is created over the Runtime UDP socket

### Requirement: Temporary selected UDP sockets remain outside the Runtime owner
When UDP punching selects a temporary random-listen UDP socket, that socket SHALL NOT be treated as part of the Runtime-owned UDP owner.

The temporary selected UDP socket SHALL be owned by the selected path/session after handoff, while Runtime continues to own its long-lived UDP socket.

#### Scenario: Temporary winner does not replace Runtime UDP owner
- **WHEN** mode2 or mode4 UDP punching selects a temporary random-listen UDP socket
- **THEN** Runtime keeps its long-lived UDP owner open
- **AND** the temporary winner is handed to the selected path/session as a separate owned resource
