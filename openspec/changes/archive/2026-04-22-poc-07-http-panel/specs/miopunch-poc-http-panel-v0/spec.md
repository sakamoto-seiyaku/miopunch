## ADDED Requirements

### Requirement: HTTP panel is disabled by default and is loopback-only
The system SHALL provide an optional HTTP panel.
The panel SHALL be disabled by default.

When enabled, the panel SHALL listen only on the IPv4 loopback address `127.0.0.1`, using the configured `listen_addr` (default `127.0.0.1:27400`).

If configured to a non-loopback listen address, the system SHALL refuse to start the panel and SHALL return actionable diagnostics.

#### Scenario: Panel starts on loopback when enabled
- **WHEN** the panel is enabled
- **AND** `listen_addr` is `127.0.0.1:27400`
- **THEN** the panel listens on `127.0.0.1:27400`
- **AND** a browser can load the panel page from that address

#### Scenario: Panel refuses a non-loopback listen address
- **WHEN** the panel is enabled
- **AND** `listen_addr` is `0.0.0.0:27400`
- **THEN** the panel does not start
- **AND** the system returns actionable output explaining the loopback-only requirement

### Requirement: Panel serves an embedded single-page UI without external dependencies
The panel SHALL serve a minimal single-page UI from the panel listener.
The UI and its required assets SHALL be embedded in the binary (e.g., via `go:embed`) and SHALL NOT require any external network access (no CDN dependencies).

#### Scenario: Browser loads the UI without internet access
- **WHEN** a user opens `http://127.0.0.1:27400/` in a browser
- **THEN** the panel returns an HTML document and required assets from the local listener

### Requirement: Panel exposes a minimal `/api/v0` route set and snapshot-first SSE streams
The panel SHALL expose the following routes:
- `GET /api/v0/status`
- `GET /api/v0/peers`
- `GET /api/v0/tasks`
- `GET /api/v0/tasks/<task_id>`
- `GET /api/v0/events` (SSE)
- `GET /api/v0/tasks/<task_id>/events` (SSE)
- `GET /api/v0/tasks/<task_id>/report` (Markdown)

For both global and per-task SSE streams, the server SHALL:
- Send a `snapshot` event as the first SSE event
- Use a single JSON event body with a `kind` field to distinguish event kinds

#### Scenario: Global SSE stream is snapshot-first
- **WHEN** a client connects to `GET /api/v0/events`
- **THEN** the first SSE event body is a JSON object with `kind: "snapshot"`

### Requirement: Panel only allows whitelisted task creation kinds
The panel SHALL expose `POST /api/v0/tasks` to create tasks.
The panel SHALL only allow creating tasks of the following kinds:
- `invite`
- `join`
- `sh_attach`

If a client attempts to create any other task kind, the panel SHALL reject the request and SHALL return actionable diagnostics instructing the user to use the CLI.

#### Scenario: Creating a non-whitelisted task kind is rejected
- **WHEN** a client sends `POST /api/v0/tasks` with `kind: "ping"`
- **THEN** the panel rejects the request
- **AND** the response includes actionable diagnostics instructing the user to use the CLI

### Requirement: Panel enforces same-origin for write operations and WebSocket attach
For the following operations, the panel SHALL enforce a same-origin check:
- `POST /api/v0/tasks`
- `GET /api/v0/tasks/<task_id>/ws`

The server SHALL validate `Origin` and/or `Referer` against the panel's configured `listen_addr` origin.

When `listen_addr` is `127.0.0.1:<port>`, the server SHALL also accept the equivalent `http://localhost:<port>` origin to support users opening the panel via `localhost`.
If the origin validation fails (or required origin headers are missing), the server SHALL reject the request.

#### Scenario: Cross-origin POST is rejected
- **WHEN** a browser sends `POST /api/v0/tasks`
- **AND** the `Origin` does not match the panel origin
- **THEN** the panel rejects the request

### Requirement: Panel WebSocket is `sh_attach` only and requires `miopunch.sh.v0`
`GET /api/v0/tasks/<task_id>/ws` SHALL only be usable when the referenced task has `kind=sh_attach`.
The WebSocket handshake SHALL require negotiating subprotocol `miopunch.sh.v0` (`Sec-WebSocket-Protocol`).

#### Scenario: WebSocket handshake fails without the required subprotocol
- **WHEN** a client opens `GET /api/v0/tasks/<task_id>/ws` for a `sh_attach` task
- **AND** the client does not negotiate `miopunch.sh.v0`
- **THEN** the handshake fails
