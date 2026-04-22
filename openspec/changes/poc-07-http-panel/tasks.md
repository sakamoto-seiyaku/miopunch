## 1. Scaffolding (HTTP panel primitives)

- [x] 1.1 Add `internal/http_panel` package skeleton (server, config, mux)
- [x] 1.2 Add embedded static assets directory and `go:embed` wiring for the panel UI
- [x] 1.3 Vendor and pin `xterm.js` (+ minimal QR lib) and record licenses under `LICENSES/`

## 2. Panel listener (loopback-only)

- [x] 2.1 Add `miopunch up` options to enable the panel and configure `listen_addr` (default `127.0.0.1:27400`)
- [x] 2.2 Enforce loopback-only listen; refuse non-loopback with actionable diagnostics
- [x] 2.3 Start/stop the panel HTTP server alongside LocalAPI in `miopunch up` (graceful shutdown)
- [x] 2.4 When enabled, print the panel URL once on startup

## 3. Panel API (`/api/v0` + SSE + report)

- [x] 3.1 Implement `GET /api/v0/status`, `GET /api/v0/peers` (read-only)
- [x] 3.2 Implement `GET /api/v0/tasks` and `GET /api/v0/tasks/<task_id>` (read-only)
- [x] 3.3 Implement `GET /api/v0/events` and `GET /api/v0/tasks/<task_id>/events` as snapshot-first SSE streams
- [x] 3.4 Implement `GET /api/v0/tasks/<task_id>/report` (Markdown) backed by persisted reports
- [x] 3.5 Implement `POST /api/v0/tasks` with kind whitelist: `invite/join/sh_attach` only; reject others with “use CLI” guidance
- [x] 3.6 Enforce same-origin checks for `POST /api/v0/tasks` (Origin/Referer vs panel origin)

## 4. WebSocket attach (`sh_attach`)

- [x] 4.1 Implement `GET /api/v0/tasks/<task_id>/ws` for `kind=sh_attach` only
- [x] 4.2 Require WebSocket subprotocol negotiation: `Sec-WebSocket-Protocol: miopunch.sh.v0`
- [x] 4.3 Enforce same-origin checks for WebSocket attach

## 5. UI (Status / Invite / Join / Shell)

- [x] 5.1 Implement a single-page UI (4 tabs): `Status / Invite / Join / Shell`
- [x] 5.2 Implement a global SSE client (`/api/v0/events`) and render task/status cards from snapshot + updates
- [x] 5.3 Invite: create `invite` task and display invite code with copy + QR (no external CDN)
- [x] 5.4 Join: create `join` task, follow per-task SSE, and render stage/reason_code/facts/suggestions + report link
- [x] 5.5 Shell: create `sh_attach` task, connect WS, and render a terminal using `xterm.js` (resize + disconnect handling)

## 6. Tests & validation

- [x] 6.1 Add unit tests for task kind whitelist (`invite/join/sh_attach`) and rejection diagnostics
- [x] 6.2 Add unit tests for same-origin enforcement (POST + WS)
- [x] 6.3 Add integration tests for SSE snapshot-first and WS subprotocol requirement
- [x] 6.4 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 6.5 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 6.6 Run `bash scripts/check_no_xtcp_imports.sh`
