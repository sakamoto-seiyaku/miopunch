# miopunch-poc-v1-gui-wizard Specification

## Purpose
定义当前 POC v1 的 desktop control console：消费 headless runtime 的 snapshot / events / actions，并以四个 operator tabs 提供默认桌面入口。

## Requirements

### Requirement: Current v1 desktop console uses four operator tabs
The system SHALL implement exactly these four current v1 GUI tabs:

- Network
- Shell
- Admin
- Settings

The GUI SHALL continue consuming the runtime-owned `stage` field from `miopunch-poc-v1-headless-runtime`, but it SHALL use that field as status/evidence input rather than as the primary GUI navigation structure.

#### Scenario: Desktop flow stays on the four-tab control-console path
- **WHEN** a user runs the current v1 desktop GUI
- **THEN** the GUI presents exactly the four operator tabs
- **AND** it does not expose legacy task pages or a fixed six-page wizard as the primary shell

### Requirement: Current v1 GUI separates summary from evidence
The system SHALL present a short runtime summary and SHALL expose detailed diagnostics separately through `Evidence`.

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

### Requirement: Current v1 shell tab keeps the runtime-owned shell gate
The `Shell` tab SHALL NOT allow shell attach until one successful identity-bound `ping` or `hello` exchange has completed over the newly established session.

The GUI SHALL consume that gate state from `miopunch-poc-v1-headless-runtime` rather than defining a competing gate locally.

#### Scenario: Shell attach remains runtime-gated inside the control console
- **WHEN** a user opens the `Shell` tab before the runtime gate is satisfied
- **THEN** the GUI blocks shell attach and prompts the user to run the runtime-owned ping action first
- **AND** after the gate is satisfied, the GUI opens the websocket-backed shell stream without changing backend transport style

### Requirement: Current v1 admin tab keeps the latest invite visible and copyable
The `Admin` tab SHALL keep the latest created invite code visible after invite creation and SHALL expose a copy action for that code.

The same invite code SHALL remain visible from non-admin tabs through a secondary recent-invite surface until it is replaced by a newer invite.

#### Scenario: Invite creation leaves a reusable visible code
- **WHEN** a user creates an invite from the `Admin` tab
- **THEN** the GUI renders the invite code in a read-only field with a copy action
- **AND** the code remains accessible after the user switches to another tab

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

For peer presentation, the runtime SHALL project the single domain `DiscoverView` owned by `miopunch-poc-v1-presence-discover` into the shared daemon runtime snapshot contract.

The GUI SHALL consume the runtime-owned `DiscoverView` projection instead of re-joining roster and presence independently, and SHALL NOT reconstruct peer/discover semantics from legacy `/api/v0/desktop/state`.

The GUI SHALL NOT treat legacy task internals as its long-term source of truth for the extracted v1 path.

#### Scenario: Desktop runtime reads extracted v1 state instead of legacy internals
- **WHEN** the current v1 desktop flow renders peer, punch, session, or shell state
- **THEN** it reads typed contracts from the extracted v1 path
- **AND** legacy task internals are not the governing runtime model

#### Scenario: Peer projection uses the single presence-owned view
- **WHEN** the current v1 runtime prepares peer projection data for the shared daemon runtime snapshot contract
- **THEN** it projects the one `DiscoverView` produced by `miopunch-poc-v1-presence-discover`
- **AND** it does not rebuild peer rows by separately merging legacy desktop snapshots with roster and presence data
