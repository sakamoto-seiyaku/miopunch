# miopunch-poc-localapi-v0 Specification

## Purpose
TBD - created by archiving change poc-05-daemon-up-localapi. Update Purpose after archive.
## Requirements
### Requirement: LocalAPI v0 is IPC-only and must not expose a public TCP listener
LocalAPI v0 SHALL be served only over OS-native IPC transports:
- Linux: unix domain socket
- Windows: named pipe

The system SHALL NOT expose LocalAPI v0 as an externally reachable TCP listener.

#### Scenario: LocalAPI is reachable via IPC without a TCP port
- **WHEN** a daemon is running (`miopunch up`)
- **THEN** a client can call LocalAPI via the platform IPC address
- **AND** LocalAPI does not require binding a TCP port on a network interface

### Requirement: LocalAPI v0 requires a fixed Host header for intent validation
LocalAPI v0 SHALL require `Host: local-miopunch.localapi` on all HTTP requests.
Requests with a different Host value (or missing Host) SHALL be rejected with an actionable error response.

#### Scenario: Request with the wrong Host is rejected
- **WHEN** a client sends a LocalAPI request with `Host` not equal to `local-miopunch.localapi`
- **THEN** the server rejects the request
- **AND** the response includes actionable diagnostics

### Requirement: LocalAPI v0 exposes a frozen minimal route set
LocalAPI v0 SHALL expose the following routes under `/api/v0`:
- `GET /api/v0/status`
- `GET /api/v0/peers`
- `GET /api/v0/topology`
- `GET /api/v0/tasks`
- `GET /api/v0/tasks/<task_id>`
- `POST /api/v0/tasks`
- `GET /api/v0/events` (SSE)
- `GET /api/v0/tasks/<task_id>/events` (SSE)
- `GET /api/v0/tasks/<task_id>/report` (Markdown)
- `GET /api/v0/tasks/<task_id>/ws` (WebSocket, `sh_attach` only)

The route set remains versioned under `/api/v0`; adding `GET /api/v0/topology` is the MNT-03 diagnostic extension for mainline topology and recovery observability.

#### Scenario: A client can create and observe a task
- **WHEN** a client calls `POST /api/v0/tasks` to create a task
- **THEN** the server returns a `task_id`
- **AND** the client can observe progress via `GET /api/v0/tasks/<task_id>` and task SSE

#### Scenario: A client can query mainline topology diagnostics
- **WHEN** a client calls `GET /api/v0/topology`
- **THEN** the server returns a machine-readable topology snapshot
- **AND** the snapshot includes bootstrap, reachability, active neighbor, degree, latest attempt, and recovery evidence when available

### Requirement: SSE streams are snapshot-first and use a single JSON event shape
For both global and per-task SSE streams, the server SHALL:
- Send a `snapshot` event as the first event after the connection is established
- Use a single JSON event body with a `kind` field to distinguish event kinds
- Not require or support `Last-Event-ID` replay; reconnect MUST start from a new `snapshot`

#### Scenario: SSE connection receives snapshot first
- **WHEN** a client connects to `GET /api/v0/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`

### Requirement: WebSocket is only available for `sh_attach` tasks and requires the `miopunch.sh.v0` subprotocol
`GET /api/v0/tasks/<task_id>/ws` SHALL only be usable when the referenced task has `kind=sh_attach`.
The WebSocket handshake SHALL require `Sec-WebSocket-Protocol: miopunch.sh.v0`.

#### Scenario: WebSocket is rejected for a non-`sh_attach` task
- **WHEN** a client opens `GET /api/v0/tasks/<task_id>/ws` for a task whose kind is not `sh_attach`
- **THEN** the server rejects the request

#### Scenario: WebSocket handshake fails without the required subprotocol
- **WHEN** a client opens `GET /api/v0/tasks/<task_id>/ws` for a `sh_attach` task
- **AND** the client does not negotiate `miopunch.sh.v0`
- **THEN** the handshake fails

### Requirement: LocalAPI topology diagnostics are stable for MNT-03
`GET /api/v0/topology` SHALL return a JSON object that can be used by product CLI, lab gates, and operators without parsing daemon logs.

The response SHALL include these fields when available:
- local peer identity and `net_id`
- governance and decls head summaries
- `v4_hint` and `v6_hint`
- presence and online state
- bootstrap candidate list, selected bucket, attempted candidates, and failure reasons
- active neighbor list and degree distribution
- latest selected `attempt_path`, `data_proto`, payload evidence, and failure stop condition
- recovery evidence for revoke, offline, rejoin, broker outage, IPv6 block, and portmap block

The response MUST NOT include secret material, invite secrets, net secrets, raw private keys, or unredacted join codes.

#### Scenario: Topology snapshot is safe for artifacts
- **WHEN** an MNT-03 gate saves `GET /api/v0/topology`
- **THEN** the snapshot contains enough evidence to validate bootstrap and active neighbor behavior
- **AND** it does not contain unredacted secret material

### Requirement: CLI exposes the same topology evidence as LocalAPI
The product CLI SHALL expose topology diagnostics in JSON form using the same LocalAPI data source as `GET /api/v0/topology`.

The CLI command SHALL be suitable for lab automation and MUST NOT require direct reads from daemon private state files.

#### Scenario: CLI topology output matches LocalAPI evidence
- **WHEN** a lab gate collects CLI topology JSON and LocalAPI topology JSON for the same node
- **THEN** both outputs describe the same bootstrap, reachability, active neighbor, and recovery evidence
