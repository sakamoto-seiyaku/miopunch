## MODIFIED Requirements

### Requirement: STUN Is Not Required for Direct Paths
The system SHALL allow a session to succeed via `IPv6 direct` or `IPv4 portmap direct` even when `STUN` is not configured or STUN discovery fails.
The system SHALL require STUN-derived mapped addresses only when falling back to `IPv4 punching`.
When using the built-in STUN defaults, the system SHALL sample internal STUN endpoints with bounded concurrency and SHALL stop requesting additional internal endpoints once it has gathered enough NAT-observation data for the current view.
The system SHALL keep explicit user-provided `--stun` / `stun:` behavior deterministic and SHALL NOT apply the built-in priority or early-stop policy to that explicit list.

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

## ADDED Requirements

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
