## ADDED Requirements

### Requirement: TCP and KCP peer sessions use yamux for logical stream multiplexing
TCP 与 KCP peer transport sessions SHALL use `TLS 1.3` identity binding before exposing multiplexed logical streams.

After identity binding succeeds, the session SHALL expose logical streams via `yamux`.

#### Scenario: KCP session exposes multiplexed logical streams via yamux
- **GIVEN** traversal establishes a UDP path and the selected data protocol is KCP
- **WHEN** dataplane establishes the peer transport session
- **THEN** it creates KCP, performs TLS 1.3 identity binding, and exposes logical streams through yamux

#### Scenario: TCP session exposes multiplexed logical streams via yamux
- **GIVEN** traversal establishes a TCP path and the selected data protocol is TLS
- **WHEN** dataplane establishes the peer transport session
- **THEN** it performs TLS 1.3 identity binding and exposes logical streams through yamux

### Requirement: AcceptStream honors context cancellation without session-level deadline polling
When accepting inbound logical streams over TCP/KCP, the implementation SHALL support `context.Context` cancellation without relying on session-level shared deadline polling.

#### Scenario: AcceptStream returns ctx error when canceled
- **GIVEN** a healthy TCP/KCP peer transport session
- **WHEN** the caller cancels the context passed to `AcceptStream(ctx)`
- **THEN** `AcceptStream(ctx)` returns `ctx.Err()`

### Requirement: Logical stream close diagnostics are non-fatal
Logical stream close diagnostics SHALL be emitted as informational events.

If the underlying stream `Close()` returns an error, diagnostics SHALL include the close error as structured metadata and SHALL NOT be emitted as failure events solely due to the close error.

#### Scenario: Stream close error is recorded but not failed
- **GIVEN** a logical stream is opened successfully
- **WHEN** the stream closes and the underlying transport returns a close error
- **THEN** diagnostics include `close_err`
- **AND** the close event is not emitted as a failure event solely because of that close error

### Requirement: Inbound session remote peer id may be unknown until verified
For inbound (acceptor/serve) sessions, the session layer SHALL allow `remote_peer_id` to be empty/unknown until stream-open metadata has been authorized and the application-level hello/auth step succeeds.

The system SHALL treat `stream-open.metadata.peer_id` as a declared identity until it has been verified by the existing hello/auth mechanism.

#### Scenario: Inbound session events may omit remote peer id before verification
- **GIVEN** an inbound peer transport session is established
- **WHEN** the session emits transport diagnostics before hello/auth verification completes
- **THEN** the diagnostics MAY omit `remote_peer_id`
- **AND** the stream-open path still carries declared `peer_id` for authorization
