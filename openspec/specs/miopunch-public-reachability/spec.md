# miopunch-public-reachability Specification

## Purpose
TBD - created by archiving change p35-public-reachability. Update Purpose after archive.
## Requirements
### Requirement: P2P IP Family Override Flags
The system SHALL support short flags `-4` and `-6` on peer commands to constrain the `P2P/打洞` address family.
These flags SHALL NOT constrain signaling connectivity such as MQTT, enrollment, invitation, approval, roster lookup, or control-plane message delivery.

The system SHALL also support a long-form `--p2p-ip-family` option with at least `auto`, `v4`, and `v6` values on current POC v1 peer commands that establish or reuse P2P peer sessions.

The system SHALL reject conflicting explicit IP-family flags in the same command.

#### Scenario: Force IPv4-only P2P without constraining signaling
- **WHEN** a peer command is run with `-4`
- **THEN** it gathers and attempts only IPv4 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Force IPv6-only P2P without constraining signaling
- **WHEN** a peer command is run with `-6`
- **THEN** it gathers and attempts only IPv6 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Long-form IP family policy matches short flags
- **WHEN** a peer command is run with `--p2p-ip-family v4`
- **THEN** it behaves as though the command was run with `-4`
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Conflicting family flags fail early
- **WHEN** a peer command is run with both `-4` and `-6`
- **THEN** the command fails with an argument error
- **AND** no P2P path establishment is attempted

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

### Requirement: STUN Endpoint Scheme Prefixes
The system SHALL accept STUN endpoints in the following forms:
- `host:port`: a dual endpoint that MAY be used for both UDP and TCP STUN
- `udp://host:port`: an endpoint restricted to UDP STUN
- `tcp://host:port`: an endpoint restricted to TCP STUN

When the system performs UDP STUN sampling, it SHALL ignore `tcp://` endpoints.
When the system performs TCP STUN sampling, it SHALL ignore `udp://` endpoints.

#### Scenario: UDP sampling ignores TCP-only endpoints
- **WHEN** a STUN endpoint list contains both `tcp://host:port` and a UDP-compatible endpoint
- **AND** the system performs UDP STUN sampling
- **THEN** the `tcp://` endpoint does not cause the run to fail
- **AND** only UDP-compatible endpoints are used for UDP STUN sampling

#### Scenario: Explicit STUN config fails fast if no usable endpoints remain
- **WHEN** the user explicitly configures STUN endpoints
- **AND** after applying the UDP/TCP scheme filter, no endpoints remain usable for the configured STUN sampling protocol
- **THEN** the system fails with a configuration error

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
