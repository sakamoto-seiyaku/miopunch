# Code Review: review-current-go-code

## Summary

- Baseline commit: `770eb867c56a08d66108b822e21026c3b5f1082a`.
- Scope reviewed: current Go codebase, including `connectivity/`, `dataplane/`, `internal/`, `cmd/`, `tools/`, `stun/`, `nat/`, and Go tests.
- Review mode: findings only; no Go source, tests, runtime scripts, public APIs, or behavior were modified.
- High-level result: automated tests/vet/import checks pass, but `gofmt -d .` reports formatting diffs and manual review found several correctness/concurrency/lifecycle issues.

## Findings

### Must Fix

- [ ] `connectivity/attempt.go:86` — **Defensive/error handling**: `attemptWithPunch` dereferences `resp.P2PNetwork` before the nil check at `connectivity/attempt.go:111`, so a nil `*wire.NatHoleResp` panics instead of returning the intended `"nil NatHoleResp"` error. Impact: malformed exchange state or future callers can crash the peer during attempt setup, bypassing normal event/error reporting.
- [ ] `internal/wire/handler.go:89` — **Concurrency/data race**: `Dispatcher.RegisterHandler` writes `msgHandlers` without synchronization while `readLoop` reads the same map at `internal/wire/handler.go:72`; live call sites register after `Run`, for example `internal/coordinator/server.go:171` and `internal/peer/client.go:170`. Impact: coordinator/client control-plane traffic can race under `-race` and may hit Go's concurrent map read/write panic in production.
- [ ] `connectivity/attempt_tcp.go:371` — **Goroutine lifetime**: TCP punching worker goroutines block on `for job := range jobCh`, but if `buildTCPPunchTargets` returns at `connectivity/attempt_tcp.go:404`, `jobCh` is never closed and the workers never observe `subCtx.Done()`. Impact: invalid or malicious `tcp_candidate_addrs` can leak up to `maxConcurrency` goroutines per attempt and leave the WaitGroup path permanently stuck.
- [ ] `cmd/miopunch/sh_interactive.go:277` — **Goroutine lifetime / CLI hang**: the stdin reader goroutine blocks in `os.Stdin.Read` and `wg.Wait` at `cmd/miopunch/sh_interactive.go:303` waits for it after websocket/context cancellation. Impact: if the remote shell/websocket closes while the user is idle, the interactive CLI can hang until stdin receives input.

### Should Fix

- [ ] `internal/wire/handler.go:59` — **Error handling/observability**: `sendLoop` discards `WriteMsg` errors, while `Dispatcher.Send` only reports enqueue success. Impact: failed control-plane writes are invisible to callers, so exchange responses, hello responses, and SID notifications can be lost without a surfaced error or diagnostic event.
- [ ] `internal/coordinator/nathole_controller.go:236` — **Error handling**: the coordinator ignores both visitor and client response `Send` errors at `internal/coordinator/nathole_controller.go:236` and `internal/coordinator/nathole_controller.go:245`, then sleeps as if responses were delivered. Impact: when either side disconnects or write delivery fails, the session can be retained through the sleep window with no actionable failure signal.
- [ ] `internal/coordinator/nathole_controller.go:145` — **Security/ID generation**: `GenSid` ignores the error from `authutil.RandID`; on entropy failure it falls back to a timestamp-only SID. Impact: session IDs become predictable and collision-prone exactly when the cryptographic random source reports failure.
- [ ] `dataplane/tls_stream.go:155` — **Resource cleanup**: `convergePinnedTLS` creates/configures pinned TLS after receiving ownership of TCP candidates, but returns immediately on TLS config errors without closing candidate conns. Impact: bad `sid`/`secret_key` input or certificate generation failure can leak established TCP sockets on the TCP data-plane path.
- [ ] `event/event.go:100` — **Observability**: `Emitter.Emit` discards JSON encode/write errors. Impact: event-stream output can silently stop or become incomplete, which undermines the project requirement that failures remain observable and replayable.
- [ ] `internal/peer/client.go:309` — **Formatting/gofmt**: `gofmt -d .` reports 681 diff lines across `connectivity/stun_view_observe_tcp.go`, `connectivity/tcp_reuse.go`, `connectivity/tcp_reuse_windows.go`, `internal/peer/client.go`, `internal/peer/client_mqtt.go`, `internal/peer/visitor.go`, `internal/peer/visitor_mqtt.go`, `internal/pocacceptor/acceptor.go`, and `internal/task/poc_dial.go`. Impact: the repository is not gofmt-clean, making later review diffs noisier and weakening the mechanical quality gate.

### Nits

- None.

## Automated Checks

- [x] `export PATH=/usr/local/go/bin:$PATH` — `go version go1.25.0 linux/amd64`.
- [ ] `gofmt -d .` — not clean; diff captured in `/tmp/review-current-go-code-gofmt.diff`.
- [x] `go test ./...` — pass.
- [x] `go vet ./...` — pass.
- [x] `bash scripts/check_no_xtcp_imports.sh` — pass (`ok: no xtcp imports`).

## Skills Applied

- `$dev`
- `$go-code-review`
- `$go-concurrency`
- `$go-style-core`
- `$go-naming`
- `$go-error-handling`
- `$go-context`
- `$go-testing`
- `$go-logging`
