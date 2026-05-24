# miopunch-poc-v1-gui-wizard Specification

## Purpose
定义当前 POC v1 的 desktop wizard：固定六阶段、summary/evidence 输出契约，以及最终可运行闭环入口。

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
The runtime API at `GET /api/v1/poc/runtime` and `GET /api/v1/poc/runtime/events` SHALL preserve this structure rather than flattening it into a single text blob.

#### Scenario: Runtime API returns structured evidence
- **WHEN** the current v1 runtime exposes stage diagnostics through the desktop API
- **THEN** `Evidence.facts` and `Evidence.suggestions` remain separately addressable
- **AND** optional deeper diagnostics may be attached without changing the summary surface

### Requirement: Current v1 reason_code set is bounded
The system SHALL bound the current v1 user-facing `UserReasonCode` set to exactly these 12 values:

- `OK`
- `BAD_INPUT`
- `BROKER_UNAVAILABLE`
- `ENROLL_REJECTED`
- `DISCOVER_EMPTY`
- `PEER_OFFLINE`
- `PEER_UNTRUSTED`
- `PUNCH_FAILED`
- `SESSION_PIN_FAILED`
- `PING_FAILED`
- `SHELL_FAILED`
- `INTERNAL`

Current v1 GUI implementations MAY keep more detailed internal diagnostics, but they SHALL map the default user-facing failure surface into this bounded set.
`internal/pocv1/runtime` SHALL be the only final owner of mapping stage-local typed failures into this bounded `UserReasonCode` set.
Earlier extracted-v1 capabilities SHALL emit typed failures and evidence, but SHALL NOT each define their own competing final user-facing bucket mapping.

#### Scenario: Reason code growth is explicitly constrained
- **WHEN** a new failure category is proposed for the current v1 GUI
- **THEN** the total `reason_code` set stays at or below 12 values
- **AND** the new category replaces or merges with an existing code if necessary

### Requirement: Current v1 GUI uses a parallel runtime API
The system SHALL expose a parallel current v1 desktop runtime API at:

- `GET /api/v1/poc/runtime`
- `GET /api/v1/poc/runtime/events`

The current v1 GUI SHALL use this API as its runtime source of truth instead of aliasing the legacy desktop task snapshot API.
This API SHALL carry the structured `Evidence` contract without flattening `facts[]` or `suggestions[]`.

#### Scenario: Current v1 GUI does not bind to the legacy runtime snapshot
- **WHEN** the current v1 desktop GUI starts its runtime bootstrap and event stream
- **THEN** it reads from `/api/v1/poc/runtime` and `/api/v1/poc/runtime/events`
- **AND** it does not treat `/api/v0/desktop/state` as the governing extracted-v1 runtime contract

### Requirement: Current v1 GUI consumes typed contracts from earlier changes
The system SHALL build the current v1 desktop runtime by consuming typed contracts from:

- `miopunch-poc-v1-enroll-bootstrap`
- `miopunch-poc-v1-presence-discover`
- `miopunch-poc-v1-dial-punch`
- `miopunch-poc-v1-secure-session`
- `miopunch-poc-v1-persistence`

The GUI SHALL NOT treat legacy task internals as its long-term source of truth for the extracted v1 path.

#### Scenario: Desktop runtime reads extracted v1 state instead of legacy internals
- **WHEN** the current v1 desktop flow renders peer, punch, session, or shell state
- **THEN** it reads typed contracts from the extracted v1 path
- **AND** legacy task internals are not the governing runtime model
