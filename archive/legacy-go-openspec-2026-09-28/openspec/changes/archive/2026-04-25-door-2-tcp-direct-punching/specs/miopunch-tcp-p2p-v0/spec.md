# miopunch-tcp-p2p-v0 Specification

## Purpose
`miopunch-tcp-p2p-v0` defines Door 2 TCP peer-to-peer connectivity: TCP direct candidates, TCP hole punching (simultaneous-open with mode0..4 semantics), controlled mode2/4 port spraying with budget guardrails, and a TCP data plane based on `TLS 1.3 + stream` with session-pinned identity verification.

## ADDED Requirements

### Requirement: Session policy supports auto/udp_only/tcp_only with a fixed attempt order
The system SHALL support a session policy key:
`p2p_network=auto | udp_only | tcp_only`.

When `p2p_network=auto`, the system SHALL attempt networks in this fixed order:
`tcp6 → tcp4 → udp6 → udp4`.

When `p2p_network=udp_only`, the system SHALL attempt only UDP networks.
When `p2p_network=tcp_only`, the system SHALL attempt only TCP networks.

#### Scenario: auto uses TCP-first ordering
- **WHEN** a session runs with `p2p_network=auto`
- **THEN** the attempt phase tries `tcp6` before `tcp4`
- **AND** the attempt phase tries `tcp4` before any UDP network

#### Scenario: udp_only skips all TCP paths
- **WHEN** a session runs with `p2p_network=udp_only`
- **THEN** the attempt phase does not attempt any TCP direct or TCP punching

#### Scenario: tcp_only skips all UDP paths
- **WHEN** a session runs with `p2p_network=tcp_only`
- **THEN** the attempt phase does not attempt any UDP direct or UDP punching

### Requirement: CLI exposes -u/-t as short policy flags (POC + Lab)
The system SHALL expose short policy flags in both the POC product CLI and lab peer CLI:
- `-u` selects `p2p_network=udp_only`
- `-t` selects `p2p_network=tcp_only`

The system SHALL also expose an equivalent long flag (e.g., `--p2p-network auto|udp_only|tcp_only`).

#### Scenario: -u forces udp_only
- **WHEN** a user runs a peer session with `-u`
- **THEN** the session policy is `p2p_network=udp_only`

#### Scenario: -t forces tcp_only
- **WHEN** a user runs a peer session with `-t`
- **THEN** the session policy is `p2p_network=tcp_only`

### Requirement: tcp_only fails fast when peer lacks tcp_p2p_v0 capability
The system SHALL define a TCP Door-2 capability string: `tcp_p2p_v0`.

Peers SHALL advertise capabilities in both:
- `PeerHello.capabilities` (coordinator signaling)
- `NatHoleVisitor.capabilities` / `NatHoleClient.capabilities` (exchange for both coordinator and MQTT signaling)

When `p2p_network=tcp_only`, the system SHALL fail fast during exchange if either peer does not advertise `tcp_p2p_v0`.

#### Scenario: tcp_only rejects a peer without tcp_p2p_v0
- **GIVEN** a session is configured with `p2p_network=tcp_only`
- **AND** one peer's advertised capabilities do not include `tcp_p2p_v0`
- **WHEN** the peers run the exchange step
- **THEN** the exchange fails explicitly
- **AND** diagnostics attribute the failure to missing TCP Door-2 capability

### Requirement: TCP port convention uses P for STUN and P+100 for listen/punching
For a given session, the system SHALL select a base port `P` and a TCP listen/punching port `L=P+100`.

- TCP STUN observation SHALL bind the local TCP source port to `P`.
- TCP direct listening and TCP punching SHALL use local port `L=P+100`.

`tcp_mapped_addrs` SHALL record STUN-observed mapped addresses for the STUN port `P` (no `+100` rewrite).

The coordinator SHALL apply the `+100` offset when deriving TCP attempt targets (e.g., `tcp_candidate_addrs` and `tcp_detect_behavior.candidate_ports`), and the attempt implementation SHALL treat these as absolute ports (no additional offsetting).

#### Scenario: tcp_candidate_addrs reflect the +100 port convention
- **GIVEN** both peers provide TCP STUN mapped addresses derived from local port `P`
- **WHEN** the coordinator derives `tcp_candidate_addrs` for attempt
- **THEN** the derived dial targets use ports that are offset by `+100` from the observed mapped ports

### Requirement: Gather produces TCP candidates and TCP STUN observations (best-effort)
When `p2p_network` permits TCP (`auto` or `tcp_only`), the system SHALL attempt to gather TCP-related inputs as a bounded-time best-effort snapshot:
- `tcp_direct_addrs` for `tcp4/tcp6` at `L=P+100`
- TCP port mapping helper candidates (UPnP/NAT-PMP) for `L=P+100` when available
- TCP STUN observation bound to local port `P` and respecting endpoint scheme filters (`host:port` dual, `tcp://` TCP-only, `udp://` UDP-only)

#### Scenario: Gather includes tcp_direct_addrs
- **WHEN** a peer runs gather with `p2p_network=auto` (or `tcp_only`)
- **THEN** the gather snapshot includes `tcp_direct_addrs` when TCP listen is available

#### Scenario: Gather may omit tcp_mapped_addrs when TCP STUN is unavailable
- **WHEN** TCP STUN discovery fails within its timeout budget
- **THEN** the gather snapshot may omit `tcp_mapped_addrs`
- **AND** the system records explainable diagnostics for the failure

### Requirement: Coordinator derives TCP punching enablement and behavior (mode0..4)
When `p2p_network` permits TCP, the coordinator SHALL derive TCP attempt inputs in `NatHoleResp`, including:
- `tcp_candidate_addrs`
- `tcp_punching_enabled` and `tcp_punching_error`
- `tcp_detect_behavior` (mode0..4 semantics)

The coordinator SHALL set `tcp_punching_enabled=false` when there is insufficient TCP STUN evidence to make an explainable punching decision (e.g., fewer than 2 mapped samples per peer), and SHALL set `tcp_punching_error` to a concrete reason.

#### Scenario: tcp_punching_enabled is false when TCP STUN evidence is insufficient
- **GIVEN** at least one peer provides fewer than 2 TCP mapped address samples
- **WHEN** the coordinator produces `NatHoleResp`
- **THEN** `tcp_punching_enabled=false`
- **AND** `tcp_punching_error` explains the missing evidence

### Requirement: TCP punching uses simultaneous-open and converges to one winner
When `tcp_punching_enabled=true`, the system SHALL implement TCP punching using simultaneous-open:
both peers listen and dial within a bounded attempt budget.

The system SHALL allow multiple successful TCP connections to occur and SHALL converge to exactly one selected connection (winner) within a settle window, closing all non-winner connections.

#### Scenario: Multiple successful connections converge to one winner
- **GIVEN** more than one TCP connection can be established during punching
- **WHEN** the system completes the punching phase
- **THEN** exactly one connection is selected as the winner
- **AND** all other established connections are closed

### Requirement: mode2/4 port spraying is bounded and explainable
When the selected TCP detect mode is `mode2` or `mode4`, the system SHALL permit port spraying using:
- `SendRandomPorts` (random destination ports)
- `ListenRandomPorts` (additional local listen+dial ports)

The system SHALL enforce v0 guardrails with defaults:
`MaxConcurrency=64`, `TotalBudget=5s(auto)/10s(tcp_only)`, `DialTimeout=1500ms(auto)/2500ms(tcp_only)`, `SettleWindow=200ms`,
and initial spraying sizes:
`SendRandomPorts=128`, `ListenRandomPorts=32`.

The system SHALL emit explainable diagnostics that include the trigger reason (mode2/4), the enforced budgets, and the actual attempt scale.

#### Scenario: mode2/4 spraying uses bounded defaults
- **GIVEN** the coordinator selects `mode=2` (or `mode=4`) for TCP detect behavior
- **WHEN** the attempt executes the TCP punching phase
- **THEN** the implementation enforces the default budgets and concurrency limits
- **AND** diagnostics include the configured and effective spraying parameters

### Requirement: TCP data plane is a TLS 1.3 stream with session-pinned identity
When a session selects a TCP connectivity path, the system SHALL use a `TLS 1.3` stream for the data plane.

The TLS identity SHALL be session-pinned without adding new wire fields by deriving key material from existing inputs (`secret_key`, `sid`, `role`) using HKDF domain separation.
The system SHALL require mutual identity verification (mTLS) based on the derived pinned identity.

#### Scenario: TLS stream rejects an unexpected pinned peer identity
- **GIVEN** the expected peer identity is derived from `secret_key + sid + role`
- **WHEN** a peer presents a TLS identity that does not match the expected pinned identity
- **THEN** the TLS connection is rejected
- **AND** the session fails with explainable diagnostics

### Requirement: Lab regression covers TCP spraying and requires payload evidence
The system SHALL provide at least one NAT lab case that exercises TCP mode2/4 spraying behavior and asserts success via ordered evidence, including `transport.payload_exchanged`.

#### Scenario: TCP spraying case produces payload evidence
- **WHEN** the lab runs the TCP spraying regression case
- **THEN** the visitor event stream contains `transport.payload_exchanged`
- **AND** the ordered evidence chain includes TCP attempt/punching diagnostics consistent with mode2/4

