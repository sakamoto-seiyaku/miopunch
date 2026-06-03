# xtcp-connectivity Specification

## Purpose
`xtcp-connectivity` 定义并约束 `P2`/Door-2 阶段的连通性增强：在不引入 `relay/fallback` 的前提下，通过 `IPv6-first`、`IPv4 port mapping helpers (UPnP/NAT-PMP)`、TCP direct / TCP punching 与固定的 attempt policy，优先走更直接、更可靠的路径；同时保留 `P1 xtcp-kernel` 的 `IPv4 punching` 作为最后兜底，并对 `gather/exchange/attempt` 提供可机读可定位的可观测事件流。
## Requirements
### Requirement: UDP-Only Connectivity in P2
The system SHALL implement `P2` connectivity enhancements for `UDP` traversal.
The system SHALL additionally support `TCP direct` and `TCP hole punching` as Door-2 paths when the session policy permits them (via `p2p_network`).

The system SHALL support the session policy key:
`p2p_network=auto | udp_only | tcp_only`.

#### Scenario: udp_only does not attempt TCP
- **GIVEN** a session is configured with `p2p_network=udp_only`
- **WHEN** the peers establish a session
- **THEN** only UDP-based traversal and transports are attempted
- **AND** TCP direct and TCP punching are not attempted

#### Scenario: auto attempts TCP before UDP
- **GIVEN** a session is configured with `p2p_network=auto`
- **WHEN** the peers establish a session
- **THEN** TCP paths are attempted before UDP paths per the fixed policy order

### Requirement: IPv6-First Direct Connectivity
The system SHALL treat `IPv6` as a first-class candidate source.
Within a given network family, the system SHALL prefer `IPv6` direct connectivity over `IPv4` direct connectivity:
`tcp6` before `tcp4`, and `udp6` before `udp4`.

#### Scenario: Prefer tcp6 when available
- **GIVEN** both peers have mutually reachable `tcp6` candidates
- **WHEN** the attempt phase runs in `p2p_network=auto`
- **THEN** the system attempts `tcp6` direct connectivity before `tcp4`

#### Scenario: Prefer udp6 when available in udp_only mode
- **GIVEN** both peers have mutually reachable `udp6` candidates
- **AND** the session is configured with `p2p_network=udp_only`
- **WHEN** the attempt phase runs
- **THEN** the system attempts `udp6` direct connectivity before any `udp4` path

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
The system SHALL allow a session to succeed via direct paths (TCP direct or UDP direct) even when `STUN` is not configured or STUN discovery fails.

The system SHALL require STUN-derived mapped addresses only when falling back to punching for that network:
- UDP punching requires UDP STUN-derived `mapped_addrs`
- TCP punching requires TCP STUN-derived `tcp_mapped_addrs`

When using the built-in STUN defaults, the system SHALL sample internal STUN endpoints with bounded concurrency and SHALL stop requesting additional internal endpoints once it has gathered enough NAT-observation data for the current view.
The system SHALL keep explicit user-provided `--stun` / `stun:` behavior deterministic and SHALL NOT apply the built-in priority or early-stop policy to that explicit list.

#### Scenario: Session succeeds without STUN via direct connectivity
- **GIVEN** both peers have mutually reachable direct candidates
- **AND** the peers do not configure any STUN servers
- **WHEN** the peers establish a session
- **THEN** the session can succeed via direct connectivity
- **AND** diagnostics show STUN was not required for the selected path

#### Scenario: Session fails with a clear reason when punching is required but STUN is unavailable
- **GIVEN** the network requires punching to establish a session
- **AND** STUN is not configured or STUN discovery fails for the required network
- **WHEN** the system reaches the fallback-to-punching step
- **THEN** the attempt fails explicitly
- **AND** diagnostics explain that punching requires STUN-derived mapped addresses

#### Scenario: Built-in STUN sampling stops after enough NAT observation
- **GIVEN** the peers use the built-in STUN defaults
- **AND** one STUN view already gathered enough mapped addresses to run NAT classification
- **WHEN** additional lower-priority internal STUN endpoints are still pending
- **THEN** the system stops issuing more requests for that view
- **AND** the already gathered observation remains eligible for `selected_view` arbitration

### Requirement: Single Snapshot Candidate Exchange (No Trickle)
The system SHALL exchange candidates as a single snapshot per session attempt.
The system SHALL NOT implement incremental candidate updates (trickle candidates) in `P2`.
Built-in internal STUN sampling SHALL complete within the current gather budget and SHALL contribute at most one finalized snapshot per view to the exchange step.

#### Scenario: Late helper results are not trickled
- **GIVEN** port mapping completes after the candidate exchange has finished
- **WHEN** the session proceeds to the attempt phase
- **THEN** the system does not send a second exchange message to add the late candidate
- **AND** the system continues with the already exchanged candidate snapshot

#### Scenario: Internal STUN view sampling produces one finalized snapshot
- **GIVEN** internal STUN sampling for a view is running with multiple in-flight requests
- **WHEN** that view reaches its stop condition or its gather deadline
- **THEN** the system emits only one finalized mapped-address snapshot for that view into exchange
- **AND** the system does not trickle later STUN responses after exchange begins

### Requirement: Fixed Attempt Policy Ordering
The system SHALL implement a fixed attempt policy order that is controlled by `p2p_network`:

- `p2p_network=auto`: `tcp6 → tcp4 → udp6 → udp4`
- `p2p_network=udp_only`: `udp6 → udp4`
- `p2p_network=tcp_only`: `tcp6 → tcp4`

For each network family attempted, the system SHALL attempt `direct` connectivity before attempting `punching`.

#### Scenario: Attempt order is observable and stable in auto
- **GIVEN** a session has TCP and UDP candidates available
- **WHEN** the attempt phase starts with `p2p_network=auto`
- **THEN** the system attempts candidates in the fixed order `tcp6 → tcp4 → udp6 → udp4`
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

### Requirement: Prioritized Built-in STUN Sampling
The system SHALL maintain a deterministic priority order for built-in STUN endpoints within each `cn` and `global` view.
The system SHALL schedule higher-priority built-in endpoints before lower-priority ones.
When using the built-in STUN defaults, the system SHALL sample each view with bounded concurrency instead of fully serial execution.
The system SHALL use the same UDP socket that will later be used for punching when issuing these STUN requests.

#### Scenario: Higher-priority built-in STUN endpoints are attempted first
- **GIVEN** a built-in STUN view contains multiple endpoints with an implementation-defined priority order
- **WHEN** sampling for that view begins
- **THEN** the system schedules the first batch from the highest-priority endpoints
- **AND** lower-priority endpoints are only started if budget remains and the stop condition has not been met

#### Scenario: cn and global views do not starve each other
- **GIVEN** both `cn` and `global` built-in STUN views are enabled
- **WHEN** internal STUN sampling runs
- **THEN** both views make progress within the same gather budget
- **AND** one slow view does not prevent the other from producing its observation snapshot
