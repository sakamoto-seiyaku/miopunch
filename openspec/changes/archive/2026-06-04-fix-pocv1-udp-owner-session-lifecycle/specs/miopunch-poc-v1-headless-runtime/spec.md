## MODIFIED Requirements

### Requirement: Runtime reuses healthy sessions and retries after fatal sessions
The runtime SHALL reuse a healthy live `PeerSession` for repeated POC v1
ping/shell actions to the same peer.

If a reused session fails at the transport level, the runtime SHALL remove it
with `transport_fatal` so the next action can establish a new punched path using
the still-owned runtime UDP socket.

#### Scenario: Reverse action after a closed session can repunch
- **GIVEN** a peer session was created for a previous POC v1 action
- **AND** that session later fails with a transport-level unavailable error
- **WHEN** the user retries ping or shell to that peer
- **THEN** Runtime does not reuse the failed session
- **AND** Runtime can punch and upgrade a new session without rebinding because the UDP socket was not closed by the failed session
