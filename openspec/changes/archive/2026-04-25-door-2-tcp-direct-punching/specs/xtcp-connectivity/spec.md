## MODIFIED Requirements

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

