## ADDED Requirements

### Requirement: P2P IP Family Override Flags
The system SHALL support short flags `-4` and `-6` on peer commands to constrain the `P2P/打洞` address family.
These flags SHALL NOT constrain signaling connectivity (e.g., MQTT/coord).

#### Scenario: Force IPv4-only P2P without constraining signaling
- **WHEN** a peer is started with `-4`
- **THEN** it gathers and attempts only IPv4 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Force IPv6-only P2P without constraining signaling
- **WHEN** a peer is started with `-6`
- **THEN** it gathers and attempts only IPv6 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

### Requirement: Built-in DNS For STUN/MQTT Resolution Only
The system SHALL support a built-in DNS resolver to resolve only:
`STUN server endpoints` and `MQTT broker endpoints`.
The built-in resolver SHALL query upstream resolvers via DNS over `TCP/53`.
The system SHALL support a configurable DNS mode with the following semantics:
- `auto`: system resolver first; fallback to built-in resolver only on failure
- `on`: always use built-in resolver
- `off`: never use built-in resolver

#### Scenario: Auto fallback resolves STUN when system DNS fails
- **WHEN** a STUN endpoint is configured as a hostname
- **AND** system DNS resolution fails
- **AND** DNS mode is `auto`
- **THEN** the system resolves the hostname using the built-in resolver
- **AND** the run proceeds without requiring the user to input a raw IP address

#### Scenario: Built-in DNS is not used outside STUN/MQTT
- **WHEN** DNS mode is `on`
- **THEN** the system uses the built-in resolver for STUN/MQTT endpoints
- **AND** it does not change DNS behavior for non-STUN/MQTT networking

### Requirement: Internal STUN Defaults With cn/global Buckets
When the user does not explicitly configure STUN servers, the system SHALL use an internal default STUN list.
The internal STUN list SHALL be partitioned into `cn` and `global(!cn)` buckets.

#### Scenario: Explicit STUN disables internal STUN and cn/global arbitration
- **WHEN** the user explicitly configures STUN servers (CLI `--stun` or YAML `stun:`)
- **THEN** the system uses only the user-provided STUN servers
- **AND** the system does not use internal STUN defaults
- **AND** the system does not perform cn/global bucket arbitration

#### Scenario: Internal STUN buckets are sampled when STUN is not explicitly configured
- **WHEN** the user does not configure any STUN servers
- **THEN** the system samples both `cn` and `global` buckets (best-effort)
- **AND** it records a per-bucket observation summary for later selection

### Requirement: Deterministic View Arbitration Produces A Single Selected View
When both `cn` and `global` bucket observations are available, the system SHALL deterministically select exactly one final `selected_view` for attempt/punching.
The arbitration order SHALL be:
`availability` → `NAT feature difficulty` → `STUN RTT` → `ok_count` → `default global`.
`STUN RTT` SHALL be derived from the `STUN binding request` round-trip time.
The RTT tie threshold SHALL be `30ms`.

#### Scenario: Availability is the highest priority
- **WHEN** `cn` has no usable observation
- **AND** `global` has a usable observation
- **THEN** `selected_view` is `global`

#### Scenario: RTT is considered only when NAT difficulty ties
- **WHEN** both `cn` and `global` are available
- **AND** NAT feature difficulty is tied between `cn` and `global`
- **AND** `global` RTT is lower than `cn` RTT by more than `30ms`
- **THEN** `selected_view` is `global`

#### Scenario: Hard ties fall back to global
- **WHEN** both `cn` and `global` are available
- **AND** NAT feature difficulty is tied
- **AND** RTT difference is within `30ms`
- **AND** ok_count is tied
- **THEN** `selected_view` is `global`

### Requirement: Observability Of View Selection
The system SHALL record the final `selected_view` and the key reason for the selection.
At `debug` log level, the system SHALL record the full evidence chain for both views and each arbitration step.

#### Scenario: Debug logs include both observations and arbitration reasons
- **WHEN** log level is `debug`
- **THEN** logs include `cn` and `global` observation summaries
- **AND** logs include the ordered reasons that produced the final `selected_view`
