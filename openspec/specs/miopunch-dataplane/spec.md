# miopunch-dataplane Specification

## Purpose
`miopunch-dataplane` 定义并约束 `P3` 阶段“打洞成功后的数据面”能力边界与回归口径：在 `connectivity` / `xtcp-kernel` 已产出可用 UDP path 之后，建立数据面会话（`kcp` / `quic`），提供 `quic` 下的拥塞控制选择（`bbr` / `brutal`），并输出可机读的 `payload exchanged` 证据链与统计信息。

## Requirements
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

