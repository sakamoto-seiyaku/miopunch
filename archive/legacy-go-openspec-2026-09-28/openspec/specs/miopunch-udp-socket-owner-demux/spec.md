# miopunch-udp-socket-owner-demux Specification

## Purpose
Defines the current POC v1 UDP socket owner/demux boundary that keeps traversal packets and KCP session traffic on the correct runtime-owned or temporary UDP socket.
## Requirements
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

### Requirement: Runtime-owned UDP sockets are not closed by borrowed views
The system SHALL treat the POC v1 daemon UDP socket as owned by the runtime or its socket-owner abstraction.

Borrowed views, demux endpoints, path results, and secure-session transports SHALL NOT close the runtime UDP socket. They MAY close their own in-memory session, listener, or endpoint state.

#### Scenario: Session close leaves runtime UDP available
- **GIVEN** a runtime-owned UDP socket has been handed to punch/session code
- **WHEN** a path result, secure-session attempt, or peer session is closed
- **THEN** the runtime UDP socket remains usable
- **AND** later punch/session attempts do not fail because the same UDP pointer references a closed file descriptor

### Requirement: UDP socket owner / demux is a hard boundary
For any UDP-based session establishment, the system SHALL enforce a single UDP socket owner / demux boundary:

- NAT traversal (`gather / attempt / direct handshake / punching`) and KCP data-plane sessions MUST share the same selected local UDP socket / port mapping.
- Only the socket owner is allowed to receive UDP packets from the underlying socket (`ReadFrom*`).
- The owner SHALL demultiplex packets to:
  - traversal transactions (direct handshake / punching), and
  - KCP session traffic.

#### Scenario: Traversal and dataplane share one UDP mapping
- **GIVEN** a peer has gathered a UDP socket on a fixed local port
- **WHEN** traversal succeeds and the data plane session is established
- **THEN** traversal and data plane both reuse that same local UDP socket / port mapping
- **AND** the implementation does not open a second UDP socket solely for the data plane

### Requirement: Punching packets are tag-prefixed and KCP-demuxable
All UDP traversal packets (direct handshake + punching messages) SHALL be prefixed with a fixed tag (PunchTagV1):

- `00 4D 50 00 01` (5 bytes)

The Runtime UDP owner SHALL classify tag-prefixed traversal packets before routing non-traversal packets to the KCP packet transport view.

#### Scenario: KCP transport does not receive traversal packets
- **GIVEN** the current POC v1 Runtime UDP owner is running on a UDP socket
- **WHEN** a tag-prefixed traversal packet arrives on that socket
- **THEN** the packet is routed to traversal demux handling
- **AND** it is not delivered to KCP as session payload

### Requirement: Server accepts multiple peer sessions concurrently on one UDP port
For current POC v1 KCP transport, a server / acceptor SHALL be able to accept and serve multiple peer transport sessions concurrently on the same UDP port.

Each accepted peer transport session SHALL support multiplexing multiple logical streams (independent of how the transport implements multiplexing).

#### Scenario: Two peers can establish sessions to the same acceptor port
- **GIVEN** an acceptor listens on a single UDP port
- **WHEN** peer A establishes a peer transport session to that port
- **AND** peer B establishes a peer transport session to that same port while A remains connected
- **THEN** both sessions are established successfully
- **AND** each session can open and serve at least one logical stream

### Requirement: Traversal demux decisions are trace-diagnosable
The UDP traversal demux SHALL provide trace-level diagnostics for packet routing decisions that affect direct handshake and punching attempts.

Diagnostics SHALL avoid logging plaintext credentials, private keys, encrypted payload bytes, or full sensitive message bodies.

The trace surface SHALL make these cases distinguishable:

- tagged packet received
- traversal message decode failure
- missing or unknown transaction ID
- packet routed to an endpoint
- endpoint queue full or packet dropped
- best-effort auto-response to an unknown transaction request

#### Scenario: Unknown transaction packet is diagnosable
- **WHEN** the demux receives a valid tagged traversal request for a transaction ID with no open endpoint
- **THEN** the demux emits trace diagnostics identifying the packet as an unknown transaction
- **AND** if it sends a best-effort response, that response decision is also trace-diagnosable

#### Scenario: Routed traversal packet is diagnosable
- **WHEN** the demux receives a valid tagged traversal packet for an open endpoint
- **THEN** the demux emits trace diagnostics identifying that the packet was routed
- **AND** the endpoint receive loop can continue without direct reads from the underlying UDP socket

#### Scenario: Decode failures do not leak payloads
- **WHEN** the demux receives a tagged traversal packet that cannot be decoded
- **THEN** the demux emits trace diagnostics for the decode failure
- **AND** it does not log encrypted payload bytes or secret material
