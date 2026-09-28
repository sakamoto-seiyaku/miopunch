## MODIFIED Requirements

### Requirement: Current v1 secure session consumes a closed PathResult handoff
The system SHALL accept `PathResult` as the only dial or punch handoff into this capability.

`PathResult` SHALL already include the remote `peer_id` and remote `MemberCredential` needed for identity pinning.
The system SHALL NOT reopen roster lookup or punch selection logic inside this capability.

`PathResult` SHALL also identify how the selected UDP path is owned:

- Runtime-owned UDP path: secure session SHALL use the Runtime owner-provided KCP packet transport view.
- temporary UDP path: secure session SHALL take ownership of the selected UDP socket for the lifetime of the session attempt/session.

The secure-session layer SHALL NOT directly read from Runtime's raw `*net.UDPConn`.

#### Scenario: Session layer does not re-read remote identity inputs
- **WHEN** current v1 dial/punch hands off a successful `PathResult`
- **THEN** the session layer has the UDP path and remote identity material it needs
- **AND** it does not reread roster state before starting the fixed session recipe

#### Scenario: Runtime-owned path uses owner PacketConn
- **WHEN** current v1 secure session consumes a Runtime-owned UDP `PathResult`
- **THEN** KCP dial or accept uses the Runtime owner PacketConn view
- **AND** it does not call KCP over Runtime's raw UDPConn directly

#### Scenario: Temporary path is owned by secure session
- **WHEN** current v1 secure session consumes a temporary UDP `PathResult`
- **THEN** the session handoff owns the selected temporary UDP socket
- **AND** it closes that socket if secure-session establishment fails

### Requirement: Current v1 uses one session recipe only
The system SHALL establish current POC v1 peer sessions using exactly one recipe:

- UDP path from `PathResult`
- KCP transport
- TLS 1.3 secure channel
- yamux multiplexing

For Runtime-owned UDP paths, the KCP transport SHALL be built on the Runtime owner PacketConn view.

For temporary UDP paths, the KCP transport SHALL be built on the selected temporary UDP socket and that socket SHALL be owned by the session on success.

For `direct_ipv6`, secure-session accept SHALL allow KCP from the observed remote UDP endpoint or from a bounded set of validated peer IPv6 direct endpoints supplied by `PathResult`.

For non-`direct_ipv6` UDP paths, secure-session accept SHALL continue to require the selected remote UDP endpoint.

TLS peer identity validation SHALL remain the authority for accepting the remote peer. IPv6 endpoint matching SHALL NOT replace peer identity verification.

The system SHALL NOT branch to QUIC, TCP, or any alternative recipe inside this capability.

#### Scenario: Session upgrade follows the fixed v1 recipe
- **WHEN** a current v1 `PathResult` is handed to the session layer
- **THEN** the runtime upgrades it through KCP, then TLS 1.3, then yamux
- **AND** it does not choose a second recipe

#### Scenario: Late traversal packets do not enter KCP
- **WHEN** a Runtime-owned UDP path receives a late tag-prefixed traversal packet while KCP session establishment is starting
- **THEN** the Runtime owner routes that packet to traversal demux handling
- **AND** KCP does not consume it as KCP payload

#### Scenario: Direct IPv6 accepts a nominated peer endpoint
- **WHEN** current v1 secure session consumes a `direct_ipv6` Runtime-owned `PathResult`
- **AND** KCP accept observes a remote UDP endpoint included in the direct IPv6 handoff set
- **THEN** it accepts that KCP session for TLS peer identity validation
- **AND** it still rejects endpoints outside the handoff set

## ADDED Requirements

### Requirement: Secure session cleanup respects selected UDP ownership
The current v1 secure-session implementation SHALL close only resources owned by the session attempt/session.

It SHALL NOT close Runtime's UDP owner or Runtime-owned UDP socket when closing a failed or completed peer session.

It SHALL close temporary selected UDP sockets on failed handoff and on session close after successful handoff.

#### Scenario: Failed Runtime-owned handoff does not close Runtime UDP
- **WHEN** secure-session establishment fails for a Runtime-owned UDP path
- **THEN** the failed handoff closes KCP/TLS/yamux resources it created
- **AND** Runtime's UDP owner remains open for later punch attempts

#### Scenario: Failed temporary handoff closes temporary UDP
- **WHEN** secure-session establishment fails for a temporary selected UDP path
- **THEN** the failed handoff closes the temporary UDP socket
- **AND** Runtime's UDP owner remains open

#### Scenario: Closing a temporary session closes its selected UDP socket
- **WHEN** a secure session established over a temporary selected UDP socket is closed
- **THEN** the session close path closes that temporary UDP socket
- **AND** non-winning temporary sockets are already closed by punching cleanup
