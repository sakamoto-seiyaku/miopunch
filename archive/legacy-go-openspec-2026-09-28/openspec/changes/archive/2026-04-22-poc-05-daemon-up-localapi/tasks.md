## 1. Scaffolding (shared primitives)

- [x] 1.1 Add `internal/poc` (or equivalent) shared types for `stage/reason_code/exit_code` and the `miopunch.json.v0` envelope
- [x] 1.2 Implement `task_id` / `request_id` generator: 16B random → RFC4648 base32(raw,no-pad) → 26 chars uppercase, plus canonicalize/parse (allow spaces/dashes)
- [x] 1.3 Implement `exit_code -> HTTP status` mapping helper (per `miopunch-poc-output-contract-v0`)

## 2. LocalAPI listener (IPC-only)

- [x] 2.1 Implement Linux unix-socket listener at `/run/miopunch/localapi.sock` (system) and `$XDG_RUNTIME_DIR/miopunch/localapi.sock` (user)
- [x] 2.2 Implement Windows named-pipe listener at `\\\\.\\pipe\\miopunch\\localapi-<operator_user_sid>`
- [x] 2.3 Enforce OS ACL boundary for LocalAPI (Linux: root+operator group; Windows: LocalSystem+operator user)
- [x] 2.4 Add middleware enforcing `Host: local-miopunch.localapi` for all LocalAPI requests

## 3. LocalAPI v0 routes (resources + tasks)

- [x] 3.1 Implement `GET /api/v0/status` returning a minimal status snapshot (version/uptime/localapi mode)
- [x] 3.2 Implement `GET /api/v0/peers` returning an empty list (POC-05 placeholder) with a forward-compatible schema
- [x] 3.3 Implement `GET /api/v0/tasks` and `GET /api/v0/tasks/<task_id>` backed by an in-memory task store (sorted by `created_at`)
- [x] 3.4 Implement `POST /api/v0/tasks` accepting `kind+args` and creating a new task record (kinds: `invite/join/approve/ping/sh_ls/sh_attach/revoke_member`)
- [x] 3.5 Implement `GET /api/v0/tasks/<task_id>/report` (Markdown) generated at task completion

## 4. SSE (events)

- [x] 4.1 Implement `GET /api/v0/events` SSE stream: first event MUST be `snapshot`, then incremental events (allow re-sending `snapshot` with throttling)
- [x] 4.2 Implement `GET /api/v0/tasks/<task_id>/events` SSE stream: first event MUST be `snapshot`, then per-task events
- [x] 4.3 Implement SSE heartbeat using comment lines (e.g. `: ping`) and ensure connections are cleaned up on disconnect
- [x] 4.4 Add unit tests verifying snapshot-first and `kind`-based single JSON event shape

## 5. WebSocket (`sh_attach` contract only; stub runtime)

- [x] 5.1 Implement `GET /api/v0/tasks/<task_id>/ws` and enforce `kind=sh_attach` only
- [x] 5.2 Enforce `Sec-WebSocket-Protocol: miopunch.sh.v0` negotiation
- [x] 5.3 Implement a stub WS loop (no PTY yet): accept connection, emit actionable failure via SSE (`NOT_IMPLEMENTED`), then close

## 6. Task runtime (minimal closed loop)

- [x] 6.1 Implement task lifecycle: `running|done`, stage updates, and final result fields (`reason_code`, `exit_code`, `facts`, `suggestions`)
- [x] 6.2 Implement stub handlers for task kinds `invite/join/approve/ping/sh_ls/sh_attach/revoke_member` that exercise the output contract and stage machine
- [x] 6.3 Implement stage machine constants (8 fixed stages) and ensure tasks only emit stages from the fixed set
- [x] 6.4 Implement report writer that renders: summary + stage timeline + reason_code + facts + suggestions

## 7. `miopunch up` (daemon process)

- [x] 7.1 Implement startup probe: check both system+user LocalAPI addresses; if either reachable, exit with actionable output (single-instance rule)
- [x] 7.2 Implement stale socket/pipe cleanup: if address exists but unreachable, remove and recreate
- [x] 7.3 Start LocalAPI server, task runtime, and graceful shutdown on SIGINT/SIGTERM

## 8. CLI as LocalAPI client (output contract freeze)

- [x] 8.1 Implement LocalAPI discovery (system first, then user) and a debug-only override (`--localapi <addr>`)
- [x] 8.2 Implement `miopunch ls` using `GET /api/v0/peers` and render the human-readable failure contract on errors
- [x] 8.3 Implement command → task mapping via `POST /api/v0/tasks` for `invite/join/approve/ping/sh/sh ls/revoke` and stream progress via SSE
- [x] 8.4 Implement `--format json` for all POC commands producing `miopunch.json.v0` one-line output
- [x] 8.5 Add CLI smoke integration test: start daemon on a temp socket, create a task, observe SSE `done`, and assert JSON envelope fields

## 9. System service install/uninstall (contract + wiring)

- [x] 9.1 Add `github.com/kardianos/service` and implement `install-system-daemon/uninstall-system-daemon` for Linux systemd + Windows Service
- [x] 9.2 Implement stable binary path copy before registration (Linux `/usr/local/bin/miopunch`, Windows `%ProgramFiles%\\miopunch\\miopunch.exe`)
- [x] 9.3 Implement operator model: Linux operator group grant; Windows operator SID capture for pipe ACL
- [x] 9.4 Ensure `uninstall-system-daemon` does not delete state; `reset` is the only state wipe

## 10. Validation

- [x] 10.1 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 10.2 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 10.3 Run `bash scripts/check_no_xtcp_imports.sh`
