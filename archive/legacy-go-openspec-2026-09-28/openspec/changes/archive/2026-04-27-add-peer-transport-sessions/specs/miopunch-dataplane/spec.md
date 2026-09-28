## ADDED Requirements

### Requirement: Peer transport session lifecycle is independent of logical stream lifecycle
The dataplane SHALL manage a peer transport session after traversal succeeds.

Logical stream close SHALL NOT close the peer transport session. The session SHALL close only through session manager decisions such as daemon shutdown, idle timeout, identity/config change, authorization revocation, or transport fatal error.

#### Scenario: Ping stream close keeps KCP session usable
- **GIVEN** a KCP-backed peer transport session is established
- **WHEN** a ping operation writes its response and closes its logical stream
- **THEN** the KCP peer transport session is not closed solely because the ping stream closed
- **AND** a later operation can open a new logical stream while the session remains healthy

### Requirement: TCP and KCP sessions use TLS 1.3 identity binding plus yamux
TCP and KCP peer transport sessions SHALL use TLS 1.3 identity binding before exposing multiplexed logical streams over yamux.

KCP SHALL NOT rely on kcp-go block crypto as the primary security layer.

#### Scenario: KCP session exposes multiplexed logical streams
- **GIVEN** traversal establishes a UDP path and the selected data protocol is KCP
- **WHEN** dataplane establishes the peer transport session
- **THEN** it creates KCP, performs TLS 1.3 identity binding, and exposes logical streams through yamux

### Requirement: QUIC sessions use native QUIC streams
QUIC peer transport sessions SHALL use QUIC native TLS 1.3 and native QUIC streams for logical stream transport.

The system SHALL NOT wrap QUIC streams in an additional TLS layer.

#### Scenario: QUIC opens native logical stream
- **GIVEN** traversal establishes a UDP path and the selected data protocol is QUIC
- **WHEN** an operation opens a logical stream
- **THEN** the stream is backed by a native QUIC stream

### Requirement: Logical stream open carries kind and metadata
Every logical stream SHALL start with a generic stream-open envelope containing a stable kind and structured metadata.

The system SHALL authorize the stream open before processing kind-specific payload frames.

#### Scenario: Shell stream is authorized before shell payload
- **WHEN** a caller opens a shell logical stream
- **THEN** the stream-open envelope identifies the shell kind and metadata
- **AND** shell payload frames are processed only after stream authorization succeeds

### Requirement: Transport close reasons are observable
Peer transport session and logical stream closures SHALL emit diagnostics that identify the close reason.

At minimum, diagnostics SHALL distinguish idle timeout, daemon shutdown, identity/config change, authorization revocation, stream protocol error, and transport fatal error.

#### Scenario: Idle session close is diagnostic
- **WHEN** a peer transport session closes due to idle timeout
- **THEN** diagnostics identify the close reason as idle timeout
- **AND** the next operation establishes a fresh session before opening a logical stream
