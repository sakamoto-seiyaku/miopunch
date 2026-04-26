## RENAMED Requirements

FROM: `### Requirement: Coordinator derives TCP punching enablement and behavior (mode0..4)`
TO: `### Requirement: Decision boundary derives TCP punching enablement and behavior (mode0..4)`

## MODIFIED Requirements

### Requirement: TCP port convention uses P for STUN and P+100 for listen/punching
For a given session, the system SHALL select a base port `P` and a TCP listen/punching port `L=P+100`.

- TCP STUN observation SHALL bind the local TCP source port to `P`.
- TCP direct listening and TCP punching SHALL use local port `L=P+100`.

`tcp_mapped_addrs` SHALL record STUN-observed mapped addresses for the STUN port `P` (no `+100` rewrite).

The punching decision boundary SHALL apply the `+100` offset when deriving TCP attempt targets (e.g., `tcp_candidate_addrs` and `tcp_detect_behavior.candidate_ports`), and the attempt implementation SHALL treat these as absolute ports (no additional offsetting).

#### Scenario: tcp_candidate_addrs reflect the +100 port convention
- **GIVEN** both peers provide TCP STUN mapped addresses derived from local port `P`
- **WHEN** the punching decision boundary derives `tcp_candidate_addrs` for attempt
- **THEN** the derived dial targets use ports that are offset by `+100` from the observed mapped ports

### Requirement: Decision boundary derives TCP punching enablement and behavior (mode0..4)
When `p2p_network` permits TCP, the punching decision boundary SHALL derive TCP attempt inputs in `NatHoleResp`, including:
- `tcp_candidate_addrs`
- `tcp_punching_enabled` and `tcp_punching_error`
- `tcp_detect_behavior` (mode0..4 semantics)

The punching decision boundary SHALL set `tcp_punching_enabled=false` when there is insufficient TCP STUN evidence to make an explainable punching decision (e.g., fewer than 2 mapped samples per peer), and SHALL set `tcp_punching_error` to a concrete reason.

#### Scenario: tcp_punching_enabled is false when TCP STUN evidence is insufficient
- **GIVEN** at least one peer provides fewer than 2 TCP mapped address samples
- **WHEN** the punching decision boundary produces `NatHoleResp`
- **THEN** `tcp_punching_enabled=false`
- **AND** `tcp_punching_error` explains the missing evidence

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
- **GIVEN** the punching decision boundary selects `mode=2` (or `mode=4`) for TCP detect behavior
- **WHEN** the attempt executes the TCP punching phase
- **THEN** the implementation enforces the default budgets and concurrency limits
- **AND** diagnostics include the configured and effective spraying parameters
