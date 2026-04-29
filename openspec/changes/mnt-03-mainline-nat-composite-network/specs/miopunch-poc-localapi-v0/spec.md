## MODIFIED Requirements

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

## ADDED Requirements

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
