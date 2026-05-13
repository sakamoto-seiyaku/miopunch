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
- `PATCH /api/v0/desktop/config`

The route set remains versioned under `/api/v0`; `desktop/state`,
`desktop/events`, and `desktop/config` are the product-facing desktop runtime
contract for the desktop shell, while task routes remain available for
compatibility, debug, and report flows.

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

#### Scenario: A desktop client saves runtime config
- **WHEN** a desktop client calls `PATCH /api/v0/desktop/config` with valid supported fields
- **THEN** the daemon persists the desired config
- **AND** the response returns a fresh desktop runtime snapshot
- **AND** desktop events publish updated config and diagnostics state

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

The `config` subtree SHALL include desired/effective runtime config and
desktop-only preferences. It MUST NOT include secret material such as peer
secret keys, invite secrets, net secrets, private keys, raw join codes, or raw
membership bundles.

The snapshot SHALL NOT require the desktop client to parse task facts as the
primary way to derive peer sessions, config state, or diagnostics.

#### Scenario: Desktop snapshot is self-contained
- **WHEN** a desktop client requests `GET /api/v0/desktop/state`
- **THEN** the response includes the product-facing runtime subtrees needed by the desktop console
- **AND** each subtree is typed JSON instead of a stringly-typed diagnostic blob

#### Scenario: Desktop config is redacted
- **WHEN** a desktop client requests `GET /api/v0/desktop/state`
- **THEN** config state includes supported non-secret settings
- **AND** it omits secret keys, invite secrets, net secrets, private keys, and raw membership material

## ADDED Requirements

### Requirement: LocalAPI supports desktop runtime config updates
`PATCH /api/v0/desktop/config` SHALL accept partial updates for supported
desktop Settings fields and SHALL validate values before persisting them.

At minimum, invalid values for `p2p_network`, `p2p_ip_family`, `data_proto`,
`quic_cc`, broker endpoints, STUN endpoints, and log level SHALL return a
structured LocalAPI bad-request error with actionable facts and suggestions.

Valid updates SHALL persist network runtime settings to the daemon state file
and desktop-only preferences to the desktop settings file. Updates SHALL return
a fresh desktop runtime snapshot.

#### Scenario: Invalid desktop config update is rejected
- **WHEN** a desktop client submits an unsupported `p2p_network`
- **THEN** LocalAPI returns a non-2xx response with `reason_code=bad_request`
- **AND** the existing persisted config is unchanged

#### Scenario: Log level update applies immediately
- **WHEN** a desktop client saves `log_level=debug`
- **THEN** the daemon persists the preference
- **AND** new daemon log output uses the saved level without requiring daemon restart
