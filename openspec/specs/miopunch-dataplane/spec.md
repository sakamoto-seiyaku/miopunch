# miopunch-dataplane Specification

## Purpose
`miopunch-dataplane` 定义并约束 `P3` 阶段“打洞成功后的数据面”能力边界与回归口径：在 `connectivity` / `xtcp-kernel` 已产出可用 UDP 或 TCP path 之后，建立数据面会话（UDP 使用 `kcp` / `quic`，TCP 使用 `tls`），提供 `quic` 下的拥塞控制选择（`bbr` / `brutal`），并输出可机读的 `payload exchanged` 证据链与统计信息。

## Requirements
### Requirement: Post-Connectivity Data Plane Boundary
The system SHALL provide a post-connectivity `data plane` that starts only after traversal establishes a usable path (UDP or TCP).
The data plane SHALL NOT change the traversal (`gather / exchange / attempt`) policy.

#### Scenario: Data plane starts after attempt succeeds
- **GIVEN** a traversal attempt succeeds and yields a usable path (UDP or TCP)
- **WHEN** the peers enter the data plane step
- **THEN** the data plane establishes a session over that selected path
- **AND** traversal policy is not re-run or modified by the data plane

### Requirement: Data Plane Mode Selection (KCP or QUIC)
The system SHALL support selecting the UDP data plane mode between `kcp` and `quic`.

Each established session SHALL select exactly one data plane mode for the selected connectivity path:
- When the selected path is `UDP`, the data plane mode is `kcp` or `quic` (per configuration).
- When the selected path is `TCP`, the data plane mode SHALL be `tls` (a TLS 1.3 stream).

The system SHALL NOT auto-switch data plane modes after a path is selected.

#### Scenario: Select KCP as the data plane mode
- **GIVEN** the developer configures `data-proto=kcp`
- **AND** the traversal selects a usable UDP path
- **WHEN** the peers enter the data plane step
- **THEN** the session uses KCP-based transport over UDP
- **AND** diagnostics identify the selected data plane mode

#### Scenario: Select QUIC as the data plane mode
- **GIVEN** the developer configures `data-proto=quic`
- **AND** the traversal selects a usable UDP path
- **WHEN** the peers enter the data plane step
- **THEN** the session uses QUIC-based transport over UDP
- **AND** diagnostics identify the selected data plane mode

#### Scenario: TCP path uses TLS stream
- **GIVEN** the traversal selects a usable TCP path
- **WHEN** the peers enter the data plane step
- **THEN** the session uses a TLS 1.3 stream
- **AND** diagnostics identify the selected data plane mode as `tls`

### Requirement: TLS Stream Uses Session-Pinned mTLS Identity
When the selected connectivity path is `TCP`, the system SHALL establish the data plane as a `TLS 1.3` stream.
The system SHALL perform mutual identity verification (mTLS) using a session-pinned identity derived from existing inputs (`secret_key`, `sid`, and `role`) and SHALL NOT require introducing additional wire fields solely for TLS pinning.

#### Scenario: Peer rejects an unexpected pinned identity
- **GIVEN** a session where the expected pinned peer identity is derived from `secret_key + sid + role`
- **WHEN** a peer presents a TLS identity that does not match the expected pinned identity
- **THEN** the TLS connection is rejected
- **AND** diagnostics attribute the failure to identity verification

### Requirement: Peer transport session lifecycle is independent of logical stream lifecycle
The dataplane SHALL manage a per-peer transport session after traversal succeeds.

The session SHALL support per-operation logical streams. Closing a logical stream SHALL NOT close the peer transport session. The session SHALL close only through session manager decisions such as daemon shutdown, idle timeout, identity/config change, authorization revocation, or transport fatal error.

#### Scenario: Ping stream close keeps KCP session usable
- **GIVEN** a KCP-backed peer transport session is established
- **WHEN** a ping operation writes its response and closes its logical stream
- **THEN** the KCP peer transport session is not closed solely because the ping stream closed
- **AND** a later operation can open a new logical stream while the session remains healthy

### Requirement: TCP and KCP sessions use TLS 1.3 identity binding plus yamux
TCP and KCP peer transport sessions SHALL use TLS 1.3 identity binding before exposing multiplexed logical streams over `yamux`.

KCP SHALL NOT rely on kcp-go block crypto as the primary security layer.

After identity binding succeeds, the session SHALL expose logical streams via `yamux`.

#### Scenario: KCP session exposes multiplexed logical streams
- **GIVEN** traversal establishes a UDP path and the selected data protocol is KCP
- **WHEN** dataplane establishes the peer transport session
- **THEN** it creates KCP, performs TLS 1.3 identity binding, and exposes logical streams through `yamux`

#### Scenario: TCP session exposes multiplexed logical streams
- **GIVEN** traversal establishes a TCP path and the selected data protocol is TLS
- **WHEN** dataplane establishes the peer transport session
- **THEN** it performs TLS 1.3 identity binding and exposes logical streams through `yamux`

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

### Requirement: AcceptStream honors context cancellation without session-level deadline polling
When accepting inbound logical streams over TCP/KCP, the implementation SHALL support `context.Context` cancellation without relying on session-level shared deadline polling.

#### Scenario: AcceptStream returns ctx error when canceled
- **GIVEN** a healthy TCP/KCP peer transport session
- **WHEN** the caller cancels the context passed to `AcceptStream(ctx)`
- **THEN** `AcceptStream(ctx)` returns `ctx.Err()`

### Requirement: Transport close reasons are observable
Peer transport session and logical stream closures SHALL emit diagnostics that identify the close reason.

At minimum, diagnostics SHALL distinguish idle timeout, daemon shutdown, identity/config change, authorization revocation, stream protocol error, and transport fatal error.

#### Scenario: Idle session close is diagnostic
- **WHEN** a peer transport session closes due to idle timeout
- **THEN** diagnostics identify the close reason as idle timeout
- **AND** the next operation establishes a fresh session before opening a logical stream

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

### Requirement: Exchange Enforces Data Plane Consistency
For a given session, both peers SHALL use the same data plane selection for:
`data-proto`, and when `data-proto=quic`, also `quic-cc` (and `up/down` limits when `quic-cc=brutal`).

If the peers are not consistent, the system SHALL fail during the `exchange` step (before `attempt`)
and SHALL emit diagnostics that attribute the failure to data plane mismatch.

#### Scenario: Exchange fails when data-proto mismatches
- **GIVEN** peer A configures `data-proto=kcp`
- **AND** peer B configures `data-proto=quic`
- **WHEN** the peers run the exchange step
- **THEN** the exchange fails explicitly
- **AND** diagnostics identify a data plane mismatch

#### Scenario: Exchange fails when QUIC CC mismatches
- **GIVEN** both peers configure `data-proto=quic`
- **AND** peer A configures `quic-cc=bbr`
- **AND** peer B configures `quic-cc=brutal`
- **WHEN** the peers run the exchange step
- **THEN** the exchange fails explicitly
- **AND** diagnostics identify a QUIC CC mismatch

### Requirement: QUIC Congestion Control Mode Selection (BBR or Brutal)
When `data-proto=quic`, the system SHALL support selecting the QUIC congestion control mode between `bbr` and `brutal`.
If not specified, the QUIC congestion control mode SHALL default to `bbr`.

#### Scenario: QUIC defaults to BBR
- **GIVEN** the developer configures `data-proto=quic` without specifying the CC mode
- **WHEN** the peers enter the data plane step
- **THEN** QUIC uses `bbr`
- **AND** diagnostics identify the QUIC CC mode as `bbr`

#### Scenario: QUIC uses brutal when configured
- **GIVEN** the developer configures `data-proto=quic` and `quic-cc=brutal`
- **WHEN** the peers enter the data plane step
- **THEN** QUIC uses `brutal`
- **AND** diagnostics identify the QUIC CC mode as `brutal`

### Requirement: Brutal Requires Explicit Bandwidth Limits
When `quic-cc=brutal`, the system SHALL require explicit `up` and `down` bandwidth limits.
The system SHALL NOT attempt to auto-detect bandwidth limits in `P3`.

#### Scenario: brutal rejects missing bandwidth limits
- **GIVEN** the developer configures `data-proto=quic` and `quic-cc=brutal`
- **WHEN** the bandwidth limits are missing
- **THEN** the session fails explicitly with a configuration error
- **AND** diagnostics identify the failure as a brutal configuration issue

### Requirement: QUIC Stack Must Support Brutal (HY2 Fork)
To support `brutal`, the system SHALL build QUIC support using the QUIC implementation used by the latest `HY2` release.
The system SHALL pin the QUIC fork version and only upgrade it via an explicit change or a critical fix.

#### Scenario: QUIC stack is pinned to the selected HY2 release
- **GIVEN** the system is built with QUIC enabled
- **WHEN** the developer inspects dependency versions
- **THEN** the QUIC implementation is pinned to the selected HY2 release fork

### Requirement: Data Plane Observability
The system SHALL expose machine-readable diagnostics for the data plane step, including:
handshake start, handshake success/failure, payload exchanged evidence, and transport statistics.

#### Scenario: Successful session produces payload exchanged evidence
- **GIVEN** the traversal attempt succeeds and the data plane establishes a session
- **WHEN** the peers exchange an application payload
- **THEN** diagnostics include an explicit payload exchanged evidence event
- **AND** transport statistics are recorded for the run
