# miopunch-poc-v1-gui-wizard Specification

## Purpose
定义当前 POC v1 的 desktop wizard：消费 headless runtime 的固定六阶段 GUI 呈现与默认桌面入口。

## ADDED Requirements

### Requirement: Current v1 desktop flow uses exactly six stages
The system SHALL implement exactly these six current v1 GUI stages:

- Network
- Enroll
- Discover
- Punch
- SecureSession
- Shell

The `SecureSession` stage SHALL NOT transition to `Shell` until one successful identity-bound `ping` or `hello` exchange has completed over the newly established session.
The GUI SHALL consume that gate state from `miopunch-poc-v1-headless-runtime` rather than defining a competing gate locally.

#### Scenario: Desktop flow stays on the fixed six-stage path
- **WHEN** a user runs the current v1 desktop flow from network setup to shell
- **THEN** the GUI presents exactly the six fixed stages
- **AND** it does not insert extra default-path half-stages

### Requirement: Current v1 GUI separates summary from evidence
The system SHALL present a `UserSummary` of at most three lines per stage and SHALL expose detailed diagnostics separately through `Evidence`.

#### Scenario: User guidance stays short while diagnostics remain available
- **WHEN** a current v1 stage progresses, succeeds, or fails
- **THEN** the default GUI shows a short summary of at most three lines
- **AND** detailed diagnostics are available separately through `Evidence`

### Requirement: Current v1 Evidence is structured and preserved
The system SHALL represent stage diagnostics as structured `Evidence`.

`Evidence` SHALL contain at least:

- `facts[]`
- `suggestions[]`
- optional stage-specific diagnostic details

`facts[]` SHALL capture concrete observed facts.
`suggestions[]` SHALL contain at least one concrete user action when a stage is blocked or failed.
The shared daemon `localapi` RPC and runtime-event stream contracts SHALL preserve this structure rather than flattening it into a single text blob, and the GUI SHALL consume that structure without redefining it.

#### Scenario: Runtime API returns structured evidence
- **WHEN** the current v1 runtime exposes stage diagnostics through the desktop API
- **THEN** `Evidence.facts` and `Evidence.suggestions` remain separately addressable
- **AND** optional deeper diagnostics may be attached without changing the summary surface

### Requirement: Current v1 GUI consumes the bounded runtime reason_code surface
The system SHALL consume the bounded current v1 user-facing `reason_code` surface provided by `miopunch-poc-v1-headless-runtime`.

Current v1 GUI implementations MAY keep richer GUI-local presentation detail, but they SHALL NOT define a competing final user-facing reason-code taxonomy.

#### Scenario: GUI does not fork the runtime failure taxonomy
- **WHEN** the current v1 runtime exposes a user-facing `reason_code`
- **THEN** the GUI presents that bounded failure surface
- **AND** it does not replace it with a second independent reason-code contract

### Requirement: Current v1 GUI uses the shared daemon localapi contract
The system SHALL expose the current v1 desktop runtime through the shared daemon `localapi` contract.

The current v1 GUI SHALL use that RPC plus dedicated stream channels as its runtime source of truth instead of aliasing the legacy desktop task snapshot API or routing through a CLI bridge.
This contract SHALL carry the structured `Evidence` contract without flattening `facts[]` or `suggestions[]`.

#### Scenario: Current v1 GUI does not bind to the legacy runtime snapshot
- **WHEN** the current v1 desktop GUI starts its runtime bootstrap and event stream
- **THEN** it reads from the shared daemon `localapi` RPC and stream contracts
- **AND** it does not treat `/api/v0/desktop/state` as the governing extracted-v1 runtime contract

### Requirement: Current v1 GUI consumes typed contracts from earlier changes
The system SHALL build the current v1 desktop flow by consuming typed contracts from:

- `miopunch-poc-v1-headless-runtime`
- `miopunch-poc-v1-enroll-bootstrap`
- `miopunch-poc-v1-presence-discover`
- `miopunch-poc-v1-dial-punch`
- `miopunch-poc-v1-secure-session`
- `miopunch-poc-v1-persistence`

For the Discover stage, the runtime SHALL project the single domain `DiscoverView` owned by `miopunch-poc-v1-presence-discover` into the shared daemon runtime snapshot contract.

The GUI SHALL consume the runtime-owned `DiscoverView` projection instead of re-joining roster and presence independently, and SHALL NOT reconstruct discover semantics from legacy `/api/v0/desktop/state`.

The GUI SHALL NOT treat legacy task internals as its long-term source of truth for the extracted v1 path.

#### Scenario: Desktop runtime reads extracted v1 state instead of legacy internals
- **WHEN** the current v1 desktop flow renders peer, punch, session, or shell state
- **THEN** it reads typed contracts from the extracted v1 path
- **AND** legacy task internals are not the governing runtime model

#### Scenario: Discover stage projects the single presence-owned view
- **WHEN** the current v1 runtime prepares Discover-stage data for the shared daemon runtime snapshot contract
- **THEN** it projects the one `DiscoverView` produced by `miopunch-poc-v1-presence-discover`
- **AND** it does not rebuild peer rows by separately merging legacy desktop snapshots with roster and presence data
