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
- `GET /api/v0/desktop/state`
- `GET /api/v0/desktop/events` (SSE)

The route set remains versioned under `/api/v0`; `desktop/state` and
`desktop/events` are the product-facing desktop runtime state contract for the
desktop shell, while task routes remain available for compatibility, debug, and
report flows.

#### Scenario: A client can create and observe a task
- **WHEN** a client calls `POST /api/v0/tasks` to create a task
- **THEN** the server returns a `task_id`
- **AND** the client can observe progress via `GET /api/v0/tasks/<task_id>` and task SSE

#### Scenario: A client can query mainline topology diagnostics
- **WHEN** a client calls `GET /api/v0/topology`
- **THEN** the server returns a machine-readable topology snapshot
- **AND** the snapshot includes bootstrap, reachability, active neighbor, degree, latest attempt, and recovery evidence when available

#### Scenario: A desktop client can bootstrap product runtime state in one call
- **WHEN** a desktop client calls `GET /api/v0/desktop/state`
- **THEN** the server returns one product-facing desktop runtime snapshot
- **AND** the snapshot includes a monotonic `rev`
- **AND** the client does not need to merge separate `status`, `peers`, `topology`, and `tasks` routes to establish initial runtime state

### Requirement: SSE streams are snapshot-first and use a single JSON event shape
For global, per-task, and desktop SSE streams, the server SHALL:
- Send a `snapshot` event as the first event after the connection is established
- Use a single JSON event body with a `kind` field to distinguish event kinds
- Not require or support `Last-Event-ID` replay; reconnect MUST start from a new `snapshot`

For task SSE streams:
- Treat task update events as coalesced state notifications, not as a lossless replay log
- Include the current task snapshot on task update events when a task is available

For desktop SSE streams:
- Carry `base_rev` and `rev` on every non-snapshot event
- Use typed product-state updates rather than refetch hints
- Allow clients to detect revision gaps and fall back to `GET /api/v0/desktop/state`

Clients SHALL NOT rely on receiving every intermediate task `stage`, `fact`, or
`diagnosis` event. Clients that need reliable task output SHALL merge the
`task` snapshot from update events or refetch the task after observing
completion.

Desktop clients SHALL treat `desktop/events` as the primary live runtime feed
for product UI state and SHALL use `desktop/state` for bootstrap and resync.

#### Scenario: SSE connection receives snapshot first
- **WHEN** a client connects to `GET /api/v0/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`

#### Scenario: Coalesced task events still carry final task output
- **WHEN** a task emits several rapid update events and the stream coalesces them
- **THEN** the latest task update event includes the current task snapshot
- **AND** a client can recover final task facts and suggestions from that snapshot

#### Scenario: Desktop event stream carries ordered runtime state updates
- **WHEN** a desktop client connects to `GET /api/v0/desktop/events`
- **THEN** the first SSE event is a JSON object with `kind: "snapshot"`
- **AND** later events carry `base_rev` and `rev`
- **AND** each later event updates one typed runtime subtree without requiring a client-side refetch

### Requirement: Desktop runtime state is product-facing and revisioned
`GET /api/v0/desktop/state` SHALL return a product-facing desktop runtime
snapshot that is safe for the desktop shell to treat as authoritative runtime
state.

The snapshot SHALL include:
- `rev`
- `status`
- `topology`
- `tasks`
- `peer_sessions`
- `shell_sessions`
- `config`
- `diagnostics`
- `approval_requests`

The snapshot SHALL NOT require the desktop client to parse task facts as the
primary way to derive peer sessions, config state, or diagnostics.

#### Scenario: Desktop snapshot is self-contained
- **WHEN** a desktop client requests `GET /api/v0/desktop/state`
- **THEN** the response includes the product-facing runtime subtrees needed by the desktop console
- **AND** each subtree is typed JSON instead of a stringly-typed diagnostic blob

#### Scenario: Desktop snapshot reserves workflow state for follow-up changes
- **WHEN** a desktop client requests `GET /api/v0/desktop/state` before richer approval workflow support exists
- **THEN** the response still includes `approval_requests`
- **AND** the field is present as a typed list even when it is empty

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

### Requirement: LocalAPI exposes typed approval request runtime state
`GET /api/v0/desktop/state` SHALL include approval requests as typed JSON objects suitable for desktop review workflows.

Each approval request object SHALL include at minimum: `approve_task_id`, `invite_id`, `request_msg_id`, `member_peer_id`, `status`, `created_at`, and `updated_at` when available.

Approval request objects MUST NOT include invite secrets, net secrets, private keys, decrypted membership bundles, or raw encrypted payloads.

Approval request objects MUST NOT include private restart decision material such as invite brokers, reply topics, validated join request bodies, or member public key payloads.

Desktop SSE SHALL publish `approval_requests.replace` when approval request state changes.

#### Scenario: Pending approval appears in desktop state
- **WHEN** an explicit-review approve task records a pending join request
- **THEN** `GET /api/v0/desktop/state` includes that request in `approval_requests`
- **AND** the request can be addressed by `approve_task_id` and `request_msg_id`

#### Scenario: Approval updates are streamed
- **WHEN** an approval request is created, approved, rejected, or expires
- **THEN** the desktop event stream emits an `approval_requests.replace` update
- **AND** the update does not expose secret material

### Requirement: LocalAPI supports approval decisions through task creation
`POST /api/v0/tasks` SHALL support task kind `approve_decision`.

The `approve_decision` task args SHALL include `approve_task_id`, `request_msg_id`, and `decision`.

The task SHALL fail without side effects when the referenced approve task or request does not exist, when the decision value is invalid, or when the request has already reached a conflicting terminal decision.

The task SHALL support pending requests that were persisted before daemon restart and SHALL NOT require the referenced `approve` task to still be active.

#### Scenario: Desktop client submits an approve decision
- **GIVEN** LocalAPI has a pending approval request
- **WHEN** a client creates an `approve_decision` task with `decision="approve"`
- **THEN** the task applies the approval decision to the referenced request
- **AND** task state reports the final decision result

#### Scenario: Desktop client submits a decision after daemon restart
- **GIVEN** LocalAPI has a persisted pending approval request from before daemon restart
- **WHEN** a client creates an `approve_decision` task for that request
- **THEN** the task applies the approval decision without an active `approve` runtime
- **AND** desktop state reports the final decision result

#### Scenario: Invalid decision is rejected
- **WHEN** a client creates an `approve_decision` task with an invalid decision value
- **THEN** the task fails without changing any approval request
- **AND** diagnostics identify the invalid decision
