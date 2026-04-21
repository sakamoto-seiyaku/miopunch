## 1. State + join code (POC v0 minimal)

- [x] 1.1 Add `internal/pocstate` for state dirs + load/save JSON state
- [x] 1.2 Implement join code encode/decode (`miopunch.join.v0`) and persist peer entries
- [x] 1.3 Implement `invite` task: create or re-output the local acceptor config as a join code
- [x] 1.4 Implement `join` task: import a join code and add/update a peer entry

## 2. Shell protocol + data-plane stream

- [x] 2.1 Add `internal/shellproto` framing (DATA/JSON) with streaming read/write helpers
- [x] 2.2 Extend `dataplane/` with stream dial/serve helpers (QUIC + KCP) for long-lived sessions
- [x] 2.3 Add heartbeat support (interval=15s) for idle shell sessions

## 3. Targets + connectors (tmux)

- [x] 3.1 Implement Linux `local` target: list tmux sessions and attach via PTY (`tmux new -A -s <session>`)
- [x] 3.2 Implement Windows connectors (build-tagged): `wsl:<distro>` and `ssh:<name>` PTY attach + session listing
- [x] 3.3 Implement target resolution rules and stable error mapping (`SH_TARGET_*`)

## 4. Single-writer lock (POC v0)

- [x] 4.1 Add a lock manager keyed by `(peer,target,session)` with `ttl=60s`
- [x] 4.2 Wire activity touch (any frame / ping-pong) and ensure TTL expiry releases lock
- [x] 4.3 Map lock conflict to `reason_code=SH_IN_USE` and `exit_code=6`

## 5. Controlled-side acceptor (daemon background service)

- [x] 5.1 Start acceptor loop from `miopunch up` when local acceptor config exists (or becomes available)
- [x] 5.2 Implement acceptor dispatch for `ping`, `sh_ls`, `sh_attach` based on the first JSON control message
- [x] 5.3 Implement `sh_attach` server side: lock + tmux attach + I/O bridge + resize handling

## 6. Visitor-side tasks (LocalAPI task runtime)

- [x] 6.1 Define JSON args for `invite/join/ping/sh_ls/sh_attach` and validate at task start
- [x] 6.2 Implement `ping` task: mqtt signaling + punching + data-plane exchange
- [x] 6.3 Implement `sh_ls` task: connect and request targets/sessions, render results into facts
- [x] 6.4 Implement `sh_attach` task: connect + capability handshake + bridge LocalAPI WS ↔ data-plane stream

## 7. LocalAPI WS + CLI interactive sh

- [x] 7.1 Extend LocalAPI WS handler to hand off the WebSocket connection to the running `sh_attach` task
- [x] 7.2 Update CLI to pass args for `ping/sh/sh ls/invite/join` to `POST /api/v0/tasks`
- [x] 7.3 Implement interactive `miopunch sh`: raw mode, byte stream forwarding, and resize messages

## 8. Tests + validation

- [x] 8.1 Unit tests for `internal/shellproto` framing and control JSON handling
- [x] 8.2 Unit tests for single-writer lock TTL/expiry behavior
- [x] 8.3 Integration test: LocalAPI WS attach drives `sh_attach` task lifecycle (no real network)
- [x] 8.4 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 8.5 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 8.6 Run `bash scripts/check_no_xtcp_imports.sh`
