# miopunch-public-reachability Specification

## Purpose
Defines current POC v1 public-network reachability controls for P2P IP-family policy, P2P network policy, DNS fallback for STUN/MQTT hostnames, STUN endpoint syntax, and ordinary built-in STUN defaults.

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

### Requirement: P2P Network Override Flags
The system SHALL support short flags `-u` and `-t` on current POC v1 peer commands that establish or reuse P2P peer sessions.

The `-u` flag SHALL select `p2p_network=udp_only`.
The `-t` flag SHALL select `p2p_network=tcp_only`.
The system SHALL also support a long-form `--p2p-network` option with at least `auto`, `udp_only`, and `tcp_only` values.

Current POC v1 is UDP-only. When `p2p_network=tcp_only` is requested, the system SHALL fail with an explicit unsupported-path result and SHALL NOT silently run UDP fallback.

#### Scenario: UDP-only policy is accepted
- **WHEN** a current POC v1 peer command is run with `-u`
- **THEN** peer session establishment uses UDP direct-first and UDP punching fallback
- **AND** signaling remains unconstrained by the P2P network policy

#### Scenario: TCP-only policy is rejected in current POC v1
- **WHEN** a current POC v1 peer command is run with `-t`
- **THEN** peer session establishment fails with an unsupported-path result
- **AND** the system does not silently run UDP path establishment

#### Scenario: Conflicting P2P network flags fail early
- **WHEN** a peer command is run with both `-u` and `-t`
- **THEN** the command fails with an argument error
- **AND** no P2P path establishment is attempted

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

### Requirement: Internal STUN Defaults For Current POC v1
When the user does not explicitly configure STUN servers, the system SHALL use the current POC v1 internal default STUN endpoint list.

The current POC v1 internal list SHALL be treated as one ordinary best-effort STUN source set for UDP mapped address discovery.

The system SHALL NOT require cn/global bucket arbitration for current POC v1 path establishment.

#### Scenario: Explicit STUN disables internal STUN
- **WHEN** the user explicitly configures STUN servers (CLI `--stun` or YAML `stun:`)
- **THEN** the system uses only the user-provided STUN servers
- **AND** the system does not use internal STUN defaults

#### Scenario: Internal STUN list is sampled when STUN is not explicitly configured
- **WHEN** the user does not configure any STUN servers
- **THEN** the system samples the internal STUN endpoint list best-effort
- **AND** current POC v1 does not require cn/global selected-view arbitration evidence

### Requirement: Observability Of STUN Discovery
The system SHALL record STUN discovery outcomes needed to diagnose current POC v1 UDP path establishment.

At `debug` log level, the system SHALL record which configured or internal STUN endpoints were attempted and whether usable mapped addresses were gathered.

#### Scenario: Debug logs include STUN attempt results
- **WHEN** log level is `debug`
- **THEN** logs include STUN endpoint attempt results and mapped address availability
- **AND** logs do not require cn/global arbitration reasons for current POC v1
