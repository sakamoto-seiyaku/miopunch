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

### Requirement: Current v1 headless runtime gate is Linux-first
The system SHALL treat Linux two-node CLI execution as the required current v1 headless runtime gate.

Windows CLI execution and Windows/Linux real-machine interoperability MAY be supported when available, but they SHALL NOT be required to complete `miopunch-poc-v1-headless-runtime`.

#### Scenario: Linux CLI is the required headless runtime acceptance path
- **WHEN** the current v1 headless runtime is accepted
- **THEN** the required smoke path runs through Linux `up`, `init-network`, `invite`, `approve`, `join`, `ls`, `ping`, `sh ls`, `sh`, and `revoke`
- **AND** Windows/Linux real-machine interoperability is tracked as follow-up scope rather than a blocker for this capability

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
