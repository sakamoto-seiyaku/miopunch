# miopunch-poc-v1-headless-runtime Specification

## Purpose
定义当前 POC v1 的 headless runtime 闭环：六阶段状态机、shared daemon、`localapi` RPC、CLI 动词接线，以及 `SecureSession -> Shell` gate。

## Requirements

### Requirement: Current v1 headless runtime owns the fixed six-stage product flow
The system SHALL implement the current POC v1 headless runtime as the only runtime authority for exactly these six stages:

- `Network`
- `Enroll`
- `Discover`
- `Punch`
- `SecureSession`
- `Shell`

The system SHALL treat CLI verbs and later GUI flows as different entry surfaces into the same six-stage runtime rather than separate product models.

#### Scenario: CLI shorthand still follows the fixed six-stage runtime
- **WHEN** a user drives the current v1 product path through CLI verbs such as `join`, `ping`, and `sh`
- **THEN** the runtime still progresses through the same six fixed stages
- **AND** those verbs do not create a second parallel state model

### Requirement: Current v1 headless runtime gates Shell on identity-bound ping or hello
The system SHALL NOT transition the current v1 runtime into `Shell` until at least one successful identity-bound `ping` or `hello` exchange has completed over the newly established secure session.

The `sh` CLI flow MAY auto-progress missing stages, but it SHALL stop before attach until this gate succeeds.

#### Scenario: Shell attach is blocked before SecureSession proof
- **WHEN** a user invokes current v1 `sh`
- **AND** no successful identity-bound `ping` or `hello` has completed for the target peer session
- **THEN** the runtime does not enter `Shell`
- **AND** the user receives actionable failure output instead of an attach attempt

### Requirement: Current v1 runtime exposes one structured runtime surface
The system SHALL expose one structured current v1 runtime surface with at least these fields:

- `stage`
- `reason_code`
- `summary`
- `evidence`
- `discover_view`
- `peer_sessions`
- `shell_sessions`

`summary` SHALL remain the short default user-facing surface.
`evidence` SHALL preserve structured diagnostics, including at least `facts[]` and `suggestions[]`.

#### Scenario: Runtime snapshot preserves summary and evidence separation
- **WHEN** a current v1 runtime snapshot is produced for CLI or GUI consumers
- **THEN** the default user surface stays in `summary`
- **AND** detailed diagnostics remain separately available in structured `evidence`

### Requirement: Current v1 runtime exposes selected UDP path evidence
The current v1 runtime SHALL expose the selected UDP path in structured success and failure evidence for `ping`, `sh ls`, and `sh` flows that establish a peer session.

The exposed path value SHALL distinguish at least:

- `direct_ipv4`
- `direct_ipv6`
- `punching_ipv4`

The CLI JSON output and report output SHALL preserve this evidence so an operator can tell whether an Android/WSL demo succeeded by LAN-direct UDP or by UDP punching.

#### Scenario: Ping output reports direct UDP selection
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP direct reachability
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=direct_ipv4`

#### Scenario: Punching output reports punching selection
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP punching fallback
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=punching_ipv4`

#### Scenario: Failure evidence remains stage-locatable
- **WHEN** current v1 peer session establishment fails after trying UDP direct reachability and UDP punching
- **THEN** the failure output includes `stage`, `reason_code`, `facts`, and `suggestions`
- **AND** the facts identify candidate-pair evidence well enough to distinguish direct timeout from punching timeout

### Requirement: Current v1 runtime owns UDP owner lifecycle
The current v1 Runtime SHALL own the lifecycle of its long-lived UDP owner and the underlying Runtime UDP socket.

Runtime SHALL create, retain, and close the long-lived UDP owner as a Runtime resource.

Punch and secure-session layers MAY borrow owner-provided traversal or packet transport views, but SHALL NOT close the Runtime UDP owner.

#### Scenario: Runtime closes UDP owner only at runtime shutdown
- **WHEN** a current v1 peer session closes after a successful Runtime-owned UDP handoff
- **THEN** Runtime's UDP owner remains open
- **AND** the owner is closed only when Runtime itself shuts down or explicitly replaces the UDP owner

#### Scenario: Failed handoff leaves Runtime UDP owner usable
- **WHEN** secure-session establishment fails after a Runtime-owned UDP punch succeeds
- **THEN** Runtime keeps the UDP owner usable for the next dial or accept attempt
- **AND** subsequent local candidates do not advertise a closed UDP file descriptor

### Requirement: Runtime reuses healthy sessions and retries after fatal sessions
The runtime SHALL reuse a healthy live `PeerSession` for repeated POC v1 ping/shell actions to the same peer.

If a reused session fails at the transport level, the runtime SHALL remove it with `transport_fatal` so the next action can establish a new punched path using the still-owned runtime UDP socket.

#### Scenario: Reverse action after a closed session can repunch
- **GIVEN** a peer session was created for a previous POC v1 action
- **AND** that session later fails with a transport-level unavailable error
- **WHEN** the user retries ping or shell to that peer
- **THEN** Runtime does not reuse the failed session
- **AND** Runtime can punch and upgrade a new session without rebinding because the UDP socket was not closed by the failed session

### Requirement: Current v1 runtime exposes punch-to-secure-session failure evidence
The current v1 Runtime SHALL expose and log actionable evidence when a selected UDP path fails during secure-session handoff.

The evidence SHALL include:

- `remote_peer_id`
- `selected_path`
- selected remote UDP endpoint
- whether the selected UDP path was Runtime-owned or temporary
- secure-session error stage

#### Scenario: Accept-side secure-session failure is visible
- **WHEN** inbound punch handling selects a UDP path
- **AND** secure-session accept fails
- **THEN** Runtime logs or exposes failure evidence with the selected path and remote UDP endpoint
- **AND** the failure is distinguishable from punch failure

#### Scenario: Dial-side secure-session failure remains stage-locatable
- **WHEN** outbound punch succeeds but secure-session dial fails
- **THEN** CLI/runtime failure output identifies `SecureSession` as the failing stage
- **AND** facts include selected UDP path evidence from the preceding punch

### Requirement: Current v1 runtime reports UDP6 selected path evidence
When current v1 Runtime establishes a UDP6 direct path, it SHALL expose selected path evidence consistently with UDP4 paths.

#### Scenario: UDP6 direct path is operator-visible
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP6 direct reachability
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=direct_ipv6`

### Requirement: Android candidate diagnostics are stage-locatable
The current v1 runtime SHALL expose trace diagnostics that identify Android local candidate sources, candidate counts, and candidate filtering outcomes during peer session establishment.

Diagnostics SHALL distinguish at least Android provider enumeration, route-source derivation, STUN mapped address gathering, direct path selection, UDP punching fallback, and secure-session upgrade.

#### Scenario: Android direct candidates are visible in logs
- **WHEN** Android Control Lite starts the current v1 runtime with trace logging
- **AND** it attempts a P2P action against a Linux peer
- **THEN** logs identify the final Android direct candidate set and its source

#### Scenario: Downstream secure session failure preserves candidate evidence
- **WHEN** P2P path establishment succeeds but secure-session upgrade fails
- **THEN** logs preserve the selected path, selected endpoints, candidate source evidence, and secure-session failure stage

### Requirement: Current v1 localapi RPC is the extracted runtime contract
The system SHALL expose the current v1 runtime through `localapi` over Unix socket / named pipe.

The control plane SHALL use `JSON-RPC`.
Runtime events and shell attach SHALL use dedicated stream channels rather than being flattened into the RPC request/response flow.

The current v1 CLI and current v1 GUI SHALL use this contract as the extracted runtime source of truth instead of treating `GET /api/v0/desktop/state` as the governing source of truth.

#### Scenario: Extracted v1 runtime is not governed by the legacy desktop snapshot
- **WHEN** a current v1 client bootstraps runtime state or subscribes to runtime events
- **THEN** it reads from the shared daemon's `localapi` RPC and stream contracts
- **AND** it does not require `/api/v0/desktop/state` to govern extracted-v1 runtime semantics

### Requirement: Current v1 product build graph is free of missing legacy authorities
The system SHALL keep the current v1 product build graph free of missing legacy authority packages.

`cmd/miopunch`, `internal/localapi`, and `internal/pocv1/*` SHALL NOT require removed or legacy-only packages such as `internal/task`, `internal/pocacceptor`, `internal/pocstate`, or `internal/controlplane` to build the extracted-v1 headless runtime path.

Legacy packages MAY be consulted as behavior references or reused behind narrow plumbing adapters only when the extracted-v1 runtime remains the owner of product semantics.

#### Scenario: Product packages can be listed without missing legacy imports
- **WHEN** the current v1 headless runtime implementation is verified
- **THEN** the product package graph for `cmd/miopunch`, `internal/localapi`, and `internal/pocv1/...` resolves without missing legacy authority imports
- **AND** any remaining legacy reference is behind an explicit plumbing boundary rather than the governing runtime contract

### Requirement: Current v1 keeps explicit `up` plus automatic same-user bootstrap
The system SHALL keep `up` as the explicit daemon startup and supervision command for current v1.

Current v1 CLI and GUI clients MAY automatically bootstrap the same-user shared daemon when it is unreachable, but they SHALL still consume the resulting runtime only through `localapi`.

#### Scenario: Client auto-bootstrap still converges to one shared daemon
- **WHEN** a current v1 CLI or GUI action runs without a reachable same-user daemon
- **THEN** the client may start that daemon automatically
- **AND** subsequent control and streaming traffic still goes through the shared daemon's `localapi` contract

### Requirement: Current v1 runtime SHALL accept an explicit broker override for init-network
The runtime SHALL accept an explicit broker endpoint override when opening the daemon and SHALL persist that override into the current network bootstrap path.

If the override is present, `init-network` SHALL use it instead of starting an embedded broker.

If the override is absent, the runtime MAY continue using the embedded broker path.

#### Scenario: init-network uses the supplied broker endpoint
- **WHEN** `miopunch up` is started with a broker override
- **AND** the user runs `miopunch init-network`
- **THEN** the resulting runtime bootstrap uses the supplied broker endpoint
- **AND** it does not start an embedded broker for that network

#### Scenario: Missing broker override still allows legacy embedded broker behavior
- **WHEN** the runtime is opened without a broker override
- **AND** the user runs `miopunch init-network`
- **THEN** the runtime may start an embedded broker as before

### Requirement: Current v1 CLI preserves the existing product verb surface
The system SHALL preserve these current v1 CLI product verbs:

- `up`
- `ls`
- `init-network`
- `invite`
- `approve`
- `join`
- `ping`
- `sh ls`
- `sh`
- `revoke`

For non-interactive commands, the system SHALL preserve `--format json`, `--report`, and `--redact`.

#### Scenario: Non-interactive CLI output contract survives runtime rewiring
- **WHEN** a user runs a non-interactive current v1 CLI command after the runtime has been rewired
- **THEN** the command still supports `--format json`, `--report`, and `--redact`
- **AND** the output shape remains suitable for automation and artifact export

### Requirement: Current v1 peer actions carry per-command P2P path policy
The current v1 runtime SHALL accept per-command P2P path policy on peer actions that establish or reuse a peer session.

The policy SHALL include:

- `p2p_network`: `auto`, `udp_only`, or `tcp_only`
- `p2p_ip_family`: `auto`, `v4`, or `v6`

The current v1 CLI SHALL expose this policy for `ping`, `sh ls`, and `sh` through short flags and long options.

The runtime SHALL treat an omitted policy as `auto` and preserve existing default behavior.

#### Scenario: Ping carries IPv4-only policy into runtime
- **WHEN** a user runs current v1 `miopunch ping <peer> -4`
- **THEN** the action arguments carry `p2p_ip_family=v4`
- **AND** peer session establishment receives that policy instead of using the default family behavior

#### Scenario: Shell list carries UDP-only policy into runtime
- **WHEN** a user runs current v1 `miopunch sh ls <peer> -u`
- **THEN** the action arguments carry `p2p_network=udp_only`
- **AND** peer session establishment receives that policy instead of ignoring the CLI option

#### Scenario: Omitted policy remains automatic
- **WHEN** a user runs current v1 `miopunch ping <peer>` without `-u`, `-t`, `-4`, `-6`, `--p2p-network`, or `--p2p-ip-family`
- **THEN** the runtime uses automatic P2P path behavior
- **AND** existing default command behavior is preserved

### Requirement: Explicit P2P path policy constrains peer session reuse
The current v1 runtime SHALL NOT reuse an existing peer session for a command with explicit P2P path policy unless the existing session satisfies that policy.

If an existing session does not satisfy the explicit policy, the runtime SHALL establish a fresh peer session under the requested policy.

If no explicit P2P path policy is supplied, the runtime MAY reuse a healthy existing peer session as before.

#### Scenario: IPv4-only command does not reuse IPv6 session
- **GIVEN** a healthy peer session already exists with selected path `direct_ipv6`
- **WHEN** a user runs current v1 `miopunch ping <peer> -4`
- **THEN** the runtime does not reuse the existing IPv6 peer session
- **AND** it establishes a fresh session under IPv4-only P2P policy

#### Scenario: Default command may reuse healthy session
- **GIVEN** a healthy peer session already exists for a peer
- **WHEN** a user runs current v1 `miopunch ping <peer>` without explicit P2P path policy
- **THEN** the runtime may reuse the existing healthy peer session

### Requirement: Current v1 reports unsupported explicit TCP-only path policy
The current v1 runtime SHALL reject an explicit `tcp_only` P2P path policy with an actionable unsupported-path failure.

The runtime SHALL NOT silently fall back to UDP when the command explicitly requested `tcp_only`.

#### Scenario: TCP-only ping fails explicitly
- **WHEN** a user runs current v1 `miopunch ping <peer> -t`
- **THEN** the command fails before UDP path establishment
- **AND** the failure explains that current POC v1 does not support TCP-only P2P path establishment

#### Scenario: TCP-only shell fails explicitly
- **WHEN** a user runs current v1 `miopunch sh <peer> -t`
- **THEN** the command fails before shell attach
- **AND** the failure explains that current POC v1 does not support TCP-only P2P path establishment

### Requirement: Current v1 headless runtime gate is Linux-first
The system SHALL treat Linux two-node CLI execution as the required current v1 headless runtime gate.

Windows CLI execution and Windows/Linux real-machine interoperability MAY be supported when available, but they SHALL NOT be required to complete `miopunch-poc-v1-headless-runtime`.

#### Scenario: Linux CLI is the required headless runtime acceptance path
- **WHEN** the current v1 headless runtime is accepted
- **THEN** the required smoke path runs through Linux `up`, `init-network`, `invite`, `approve`, `join`, `ls`, `ping`, `sh ls`, `sh`, and `revoke`
- **AND** Windows/Linux real-machine interoperability is tracked as follow-up scope rather than a blocker for this capability

### Requirement: Android Control Lite validation uses rebuilt shared runtime
Android Control Lite validation SHALL rebuild the APK payload and the Linux CLI from the same source tree before judging Android/Linux P2P behavior.

Validation SHALL use fresh app data, fresh Linux state, trace logs on both sides, and SHALL verify Android-to-Linux `ping` plus `sh ls` when Android remains a control-side demo client.

#### Scenario: Rebuilt Android demo proves P2P path
- **WHEN** the Android APK and Linux CLI are rebuilt from the changed tree
- **AND** the app joins the same network as the Linux peer using fresh state
- **THEN** Android-to-Linux `ping` succeeds or logs enough stage evidence to identify the failing layer
- **AND** Android-to-Linux `sh ls` is run as the shell demo acceptance check

### Requirement: Current v1 failures remain actionable across CLI and runtime APIs
The system SHALL provide actionable failure output for current v1 runtime-driven commands and APIs.

On any failure, the output SHALL include:

- `stage`
- `reason_code`
- `facts`
- `suggestions`

When the runtime packages diagnostics as structured `evidence`, it SHALL preserve `facts` and `suggestions` without flattening them into a single blob.

#### Scenario: Runtime failure remains explainable after legacy authority removal
- **WHEN** a current v1 runtime-driven CLI action fails
- **THEN** the failure output still includes `stage`, `reason_code`, `facts`, and `suggestions`
- **AND** at least one suggestion provides a concrete user action

### Requirement: Validated peer sessions remain reusable across idle gaps
The current v1 runtime SHALL keep a healthy peer session reusable after a
successful identity-bound `ping` or `hello` exchange.

The runtime SHALL send bounded application-level keepalive traffic for such
validated sessions so that a later `sh` can reuse the existing session instead
of forcing a fresh punch after a short idle gap.

#### Scenario: Pinged session remains reusable for later sh
- **GIVEN** a healthy peer session has completed a successful `ping` or
  `hello` exchange
- **AND** no other application traffic occurs for a period shorter than the
  keepalive budget
- **WHEN** a later `sh` targets the same peer
- **THEN** the runtime can reuse the existing healthy session
- **AND** it does not need to establish a fresh session solely because of the
  idle gap

#### Scenario: Truly idle sessions still close
- **GIVEN** a peer session has not been validated by `ping` or `hello`
- **OR** the session has no traffic for longer than the dataplane idle timeout
- **WHEN** the idle timeout elapses
- **THEN** the session is still closed by the dataplane idle closer
- **AND** the next operation must establish a fresh session

### Requirement: sh ls exposes concrete targets and sessions in operator-visible success output
When `miopunch sh ls <peer>` succeeds, the system SHALL expose the concrete target names in operator-visible success output and report output.
When `miopunch sh ls <peer> <target>` succeeds, the system SHALL expose the concrete session names in operator-visible success output and report output.

#### Scenario: sh ls without a target shows the available targets
- **WHEN** a user runs `miopunch sh ls <peer>` against a Windows-controlled peer
- **THEN** the success output includes the concrete `wsl:<distro>` and `ssh:<name>` targets
- **AND** the existing line output remains unchanged

#### Scenario: sh ls with a target shows the available sessions
- **WHEN** a user runs `miopunch sh ls <peer> <target>`
- **THEN** the success output includes the concrete tmux session names for that target
- **AND** the existing line output remains unchanged
