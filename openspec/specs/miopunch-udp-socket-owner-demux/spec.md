# miopunch-udp-socket-owner-demux Specification

## Purpose
TBD - created by archiving change udp-socket-owner-demux. Update Purpose after archive.
## Requirements
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
