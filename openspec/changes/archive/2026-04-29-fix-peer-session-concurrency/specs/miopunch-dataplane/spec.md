## ADDED Requirements

### Requirement: Data plane server accepts multiple inbound peer sessions concurrently
On the controlled (server/acceptor) side, the data plane SHALL be able to accept and serve multiple inbound peer transport sessions concurrently.

The system SHALL NOT exhibit "first session monopolizes acceptor" behavior where an already-established peer session prevents other peers from establishing their own sessions and opening logical streams.

#### Scenario: Second peer can establish a session while the first session is active
- **GIVEN** peer `p2` has an active healthy peer transport session to peer `p1`
- **WHEN** another peer `p3` attempts to establish its own peer transport session to `p1`
- **THEN** `p3` can establish the session and open a logical stream successfully
- **AND** `p2` remaining connected does not require terminating `p2` to make progress

### Requirement: QUIC server uses an accept loop for inbound connections
When the selected data protocol is QUIC, the server side SHALL accept inbound QUIC connections using an accept loop.

The server SHALL NOT accept only the first connection and then close the QUIC listener for the session lifetime.

#### Scenario: QUIC server accepts multiple inbound connections
- **WHEN** a QUIC data plane server is running for a peer
- **AND** two different remote peers connect sequentially or concurrently
- **THEN** the server accepts both inbound QUIC connections
- **AND** each connection can open logical streams independently

### Requirement: KCP server uses a listener/accept model over a packet connection
When the selected data protocol is KCP, the server side SHALL support accepting multiple inbound KCP sessions over a packet connection.

The implementation SHALL NOT bind the UDP socket to a single hard-coded KCP session (e.g., fixed conv) such that only one remote peer can be served.

#### Scenario: KCP server accepts multiple inbound sessions
- **WHEN** a KCP data plane server is running for a peer
- **AND** two different remote peers connect sequentially or concurrently
- **THEN** the server can accept a session from each peer
- **AND** each accepted session can open logical streams independently

### Requirement: Authorization revocation closes existing peer sessions on observation
When a node observes an authorization revocation for a peer identity (for example, a valid `revoke_member` tombstone in its local view), it SHALL:

1. Reject new logical stream opens from that revoked peer.
2. Proactively close any existing peer transport sessions associated with that peer identity.

The system SHALL NOT require a dedicated "revoke notification" message to the revoked peer for this behavior.

#### Scenario: Revoked peer loses access immediately after tombstone observation
- **GIVEN** a node has an existing peer transport session with a peer identity
- **WHEN** the node observes a valid revocation tombstone for that identity
- **THEN** the node closes the existing peer transport session(s) for that identity
- **AND** subsequent logical stream opens from that identity are rejected with authorization/revocation diagnostics

