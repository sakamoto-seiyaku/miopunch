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

## ADDED Requirements

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
