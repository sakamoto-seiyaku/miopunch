## ADDED Requirements

### Requirement: Post-Connectivity Data Plane Boundary
The system SHALL provide a post-connectivity `data plane` that starts only after traversal establishes a usable UDP path.
The data plane SHALL NOT change the traversal (`gather / exchange / attempt`) policy.

#### Scenario: Data plane starts after attempt succeeds
- **GIVEN** a traversal attempt succeeds and yields a usable UDP path
- **WHEN** the peers enter the data plane step
- **THEN** the data plane establishes a session over that UDP path
- **AND** traversal policy is not re-run or modified by the data plane

### Requirement: Data Plane Mode Selection (KCP or QUIC)
The system SHALL support selecting the data plane mode between `kcp` and `quic`.
Each session SHALL select exactly one data plane mode and SHALL NOT auto-switch on failures.

#### Scenario: Select KCP as the data plane mode
- **GIVEN** the developer configures `data-proto=kcp`
- **WHEN** the peers enter the data plane step
- **THEN** the session uses KCP-based transport
- **AND** diagnostics identify the selected data plane mode

#### Scenario: Select QUIC as the data plane mode
- **GIVEN** the developer configures `data-proto=quic`
- **WHEN** the peers enter the data plane step
- **THEN** the session uses QUIC-based transport
- **AND** diagnostics identify the selected data plane mode

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
