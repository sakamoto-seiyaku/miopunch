## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: TLS Stream Uses Session-Pinned mTLS Identity
When the selected connectivity path is `TCP`, the system SHALL establish the data plane as a `TLS 1.3` stream.
The system SHALL perform mutual identity verification (mTLS) using a session-pinned identity derived from existing inputs (`secret_key`, `sid`, and `role`) and SHALL NOT require introducing additional wire fields solely for TLS pinning.

#### Scenario: Peer rejects an unexpected pinned identity
- **GIVEN** a session where the expected pinned peer identity is derived from `secret_key + sid + role`
- **WHEN** a peer presents a TLS identity that does not match the expected pinned identity
- **THEN** the TLS connection is rejected
- **AND** diagnostics attribute the failure to identity verification

