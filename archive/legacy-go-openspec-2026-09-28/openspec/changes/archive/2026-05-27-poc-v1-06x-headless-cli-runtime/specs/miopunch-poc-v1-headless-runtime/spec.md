# miopunch-poc-v1-headless-runtime Specification

## Purpose
定义当前 POC v1 的 headless runtime 闭环：六阶段状态机、shared daemon、`localapi` RPC、CLI 动词接线，以及 `SecureSession -> Shell` gate。

## ADDED Requirements

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
