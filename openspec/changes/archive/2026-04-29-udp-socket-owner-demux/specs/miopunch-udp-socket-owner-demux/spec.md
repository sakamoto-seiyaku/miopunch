## ADDED Requirements

### Requirement: UDP socket owner / demux is a hard boundary
For any UDP-based session establishment, the system SHALL enforce a single UDP socket owner / demux boundary:

- NAT traversal (`gather / attempt / direct handshake / punching`) and data plane sessions (QUIC / KCP) MUST share the same local UDP socket / port mapping.
- Only the socket owner is allowed to receive UDP packets from the underlying socket (`ReadFrom*`).
- The owner SHALL demultiplex packets to:
  - traversal transactions (direct handshake / punching), and
  - data plane session protocols (QUIC or KCP).

#### Scenario: Traversal and dataplane share one UDP mapping
- **GIVEN** a peer has gathered a UDP socket on a fixed local port
- **WHEN** traversal succeeds and the data plane session is established
- **THEN** traversal and data plane both reuse that same local UDP socket / port mapping
- **AND** the implementation does not open a second UDP socket solely for the data plane

### Requirement: Punching packets are tag-prefixed and non-QUIC demuxable
All UDP traversal packets (direct handshake + punching messages) SHALL be prefixed with a fixed tag (PunchTagV1):

- `00 4D 50 00 01` (5 bytes)

When QUIC is the selected data plane protocol, the QUIC socket owner SHALL read traversal packets via `quic.Transport.ReadNonQUICPacket`.

The first byte of PunchTagV1 MUST have its first and second bit set to `0`, so that `ReadNonQUICPacket` can reliably classify the packet as non-QUIC.

#### Scenario: QUIC transport exposes traversal packets via ReadNonQUICPacket
- **GIVEN** a QUIC socket owner is running on a UDP socket
- **WHEN** a tag-prefixed traversal packet arrives on that socket
- **THEN** the packet can be read via `ReadNonQUICPacket`
- **AND** it is not delivered to an established QUIC connection as QUIC payload

### Requirement: Server accepts multiple peer sessions concurrently on one UDP port
For UDP transports (QUIC or KCP), a server / acceptor SHALL be able to accept and serve multiple peer transport sessions concurrently on the same UDP port.

Each accepted peer transport session SHALL support multiplexing multiple logical streams (independent of how the transport implements multiplexing).

#### Scenario: Two peers can establish sessions to the same acceptor port
- **GIVEN** an acceptor listens on a single UDP port
- **WHEN** peer A establishes a peer transport session to that port
- **AND** peer B establishes a peer transport session to that same port while A remains connected
- **THEN** both sessions are established successfully
- **AND** each session can open and serve at least one logical stream

