# xtcp-connectivity Specification

## Purpose
`xtcp-connectivity` 定义并约束 `P2` 阶段的连通性增强（`UDP only`）：在不引入 `relay/fallback` 的前提下，通过 `IPv6-first`、`IPv4 port mapping helpers (UPnP/NAT-PMP)` 与固定的 attempt policy，优先走更直接、更可靠的路径；同时保留 `P1 xtcp-kernel` 的 `IPv4 punching` 作为最后兜底，并对 `gather/exchange/attempt` 提供可机读可定位的可观测事件流。
## Requirements
### Requirement: UDP-Only Connectivity in P2
The system SHALL implement `P2` connectivity enhancements for `UDP` traversal only.
The system SHALL NOT implement `TCP hole punching` in `P2`.

#### Scenario: P2 does not expose TCP punching
- **GIVEN** the system is built with `xtcp-connectivity` enabled
- **WHEN** a developer configures a traversal session
- **THEN** only UDP-based traversal and transports are available
- **AND** TCP hole punching is not available as a selectable path in P2

### Requirement: IPv6-First Direct Connectivity
The system SHALL treat `IPv6` as a first-class candidate source.
If a viable direct IPv6 path exists, the system SHALL prefer it over any IPv4 path.

#### Scenario: Prefer IPv6 when available
- **GIVEN** both peers have mutually reachable IPv6 addresses
- **WHEN** the peers establish a session using `xtcp-connectivity`
- **THEN** the system attempts IPv6 direct connectivity before any IPv4 helper or punching path
- **AND** the selected path is recorded in machine-readable diagnostics

### Requirement: IPv4 Port Mapping Helper Candidates (UPnP, NAT-PMP)
The system SHALL support best-effort IPv4 port mapping helpers (`UPnP`, `NAT-PMP`) as additional candidate sources.
The system SHALL NOT require `PCP` support to meet the `P2(v1)` acceptance criteria (`PCP` is deferred to a future change).
Port mapping helpers SHALL NOT block the main exchange or attempt flow.

#### Scenario: Use port mapping as an extra candidate without blocking
- **GIVEN** a peer runs in an IPv4-only environment behind a port-mapping-capable gateway
- **WHEN** the peer enters the gather phase
- **THEN** port mapping is attempted concurrently in the background
- **AND** if one or more mappings are obtained before exchange, they are included as additional direct candidates
- **AND** if a mapping is not obtained in time, the session proceeds without waiting

### Requirement: STUN Is Not Required for Direct Paths
The system SHALL allow a session to succeed via `IPv6 direct` or `IPv4 portmap direct` even when `STUN` is not configured or STUN discovery fails.
The system SHALL require STUN-derived mapped addresses only when falling back to `IPv4 punching`.

#### Scenario: Session succeeds without STUN via IPv6
- **GIVEN** both peers have mutually reachable IPv6 addresses
- **AND** the peers do not configure any STUN servers
- **WHEN** the peers establish a session using `xtcp-connectivity`
- **THEN** the session succeeds via IPv6 direct connectivity
- **AND** the diagnostics show STUN was not required for the selected path

#### Scenario: Session fails with a clear reason when punching is required but STUN is unavailable
- **GIVEN** the network requires IPv4 punching to establish a session
- **AND** STUN is not configured or STUN discovery fails
- **WHEN** the system reaches the fallback-to-punching step
- **THEN** the attempt fails explicitly
- **AND** diagnostics explain that punching requires STUN-derived mapped addresses

### Requirement: Single Snapshot Candidate Exchange (No Trickle)
The system SHALL exchange candidates as a single snapshot per session attempt.
The system SHALL NOT implement incremental candidate updates (trickle candidates) in `P2`.

#### Scenario: Late helper results are not trickled
- **GIVEN** port mapping completes after the candidate exchange has finished
- **WHEN** the session proceeds to the attempt phase
- **THEN** the system does not send a second exchange message to add the late candidate
- **AND** the system continues with the already exchanged candidate snapshot

### Requirement: Fixed Attempt Policy Ordering
The system SHALL implement a fixed attempt policy order for `P2`:
`IPv6 direct` → `IPv4 portmap direct` → `IPv4 punching (P1 xtcp kernel)`.

#### Scenario: Attempt order is observable and stable
- **GIVEN** a session has IPv6 and IPv4 candidates available
- **WHEN** the attempt phase starts
- **THEN** the system attempts candidates in the fixed order
- **AND** each attempted path is recorded with begin/end events and outcomes

### Requirement: P1 IPv4 Punching Regression Compatibility
The system SHALL preserve the behavioral baseline of the `P1` IPv4 punching kernel (`xtcp/nathole`) while adding `P2` connectivity orchestration.

#### Scenario: P1 regression remains stable under P2
- **GIVEN** the `P1` NAT lab regression matrix is available
- **WHEN** the regression suite is executed with `xtcp-connectivity` enabled
- **THEN** the `P1` punching cases continue to produce the expected outcomes
- **AND** any deviation is treated as a change proposal item with evidence and explicit approval

### Requirement: Derived P2 Path Regression Coverage
The system SHALL provide derived lab regressions for representative `P2` path selections without changing the `P0` base NAT matrix.

#### Scenario: Keep the existing representative P2 paths under strict validation
- **GIVEN** the lab runs the representative `P2` cases for `IPv6 direct`, `IPv4 portmap direct`, and `IPv4 punching fallback`
- **WHEN** the regression output is evaluated
- **THEN** each case is checked against its expected ordered evidence chain
- **AND** each successful case also requires transport payload-exchange evidence

#### Scenario: Fall back from IPv6 direct to IPv4 direct when IPv6 is present but unreachable
- **GIVEN** both peers gather `IPv6` candidates and `IPv4 portmap` candidates
- **AND** the `IPv6` path is not reachable
- **WHEN** the attempt phase runs
- **THEN** diagnostics contain `attempt.v6.start` followed by `attempt.v6.fail`
- **AND** diagnostics then contain `attempt.v4.start` followed by `attempt.v4.ok`
- **AND** the run is accepted only if the transport stage also exchanges application payload

### Requirement: Connectivity Observability
The system SHALL expose machine-readable diagnostics for `gather`, `exchange`, and `attempt` phases, including helper outcomes and failure reasons.

#### Scenario: Explain why connectivity fell back to punching
- **GIVEN** IPv6 direct and port mapping candidates are unavailable or fail
- **WHEN** the system falls back to IPv4 punching
- **THEN** the diagnostics explicitly show the failure reasons for IPv6 and port mapping
- **AND** the diagnostics show the transition to the punching phase
