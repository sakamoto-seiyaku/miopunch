## Context

`review-current-go-code` produced a findings-only report after recent Door-2/TCP/control/dataplane work. The critical pattern is not a failed high-level protocol design; it is a set of local runtime contracts that were implicit during rapid POC development:

- `wire.Dispatcher` runs asynchronous read/write loops but does not define whether handlers may be registered after `Run`, and it drops write failures.
- `event.Emitter` is the common diagnostics surface, but its API discards encoder/writer failures.
- interactive `sh_attach` treats a blocking terminal stdin read as context-cancellable, which is false.

Because this change targets a release candidate, the fix should be direct and complete enough to remove the class of failures, not only the exact reviewed line.

## Goals / Non-Goals

**Goals:**

- Fix the design-contract findings with docs/spec updates and code changes in the same change.
- Keep release behavior simple: no wire-format migration, no new signaling protocol, no broad control-plane rewrite.
- Clear the remaining reviewed implementation defects that do not require design changes.
- Add focused tests that would have caught the reviewed failures.

**Non-Goals:**

- Do not replace `wire.Dispatcher` with a new request/response framework.
- Do not introduce an async event pipeline, buffering policy, or new event transport.
- Do not redesign shell protocol frames or tmux task semantics.
- Do not mutate NAT/lab baseline scenarios just to hide failures.

## Decisions

### 1) Make `wire.Dispatcher` runtime-safe, not build-only

**Decision:** Allow handler registration before or after `Run`, and make handler lookup/update concurrency-safe. Add a terminal error surface for read/write loop failures.

**Implementation direction:**

- Protect `msgHandlers` and `defaultHandler` with a mutex.
- Copy the selected handler under lock, then invoke it after unlocking.
- Use `sync.Once` (or equivalent) for dispatcher shutdown so read/write failures cannot double-close `doneCh`.
- Preserve `Send` as asynchronous: a nil `Send` error means the message was accepted for sending, not that `WriteMsg` completed.
- Return the terminal dispatcher error from `Send` when the dispatcher is already done, and expose it through an `Err()`-style accessor for observers.
- On `WriteMsg` failure, close the dispatcher and store the error instead of discarding it.

**Alternatives considered:**

- Require all handlers before `Run`: simpler, but current peer/coordinator flow already registers by role after hello and would need unnecessary reshaping.
- Freeze registration after `Start`: cleaner long term, but too disruptive for a release fix.
- Replace dispatcher: over-scoped and risky before release.

### 2) Make `event.Emitter` return write errors directly

**Decision:** Change the emitter methods to return `error` while preserving existing call sites that intentionally ignore best-effort diagnostics.

**Implementation direction:**

- `Emit`, `Start`, `OK`, and `Fail` return encoder/write errors.
- Existing statements such as `em.Emit(ev)` may continue ignoring the result where best-effort output is acceptable.
- Critical or newly tested paths can check the returned error without needing a parallel `EmitErr` API.
- Do not make event write failure automatically fail every connectivity/dataplane operation; callers decide whether the event is required evidence for that flow.

**Alternatives considered:**

- Add `EmitErr` while leaving `Emit` void: compatible but leaves the original silent-discard footgun in place.
- Store only `LastError`: observable only if callers remember to check later.
- Add async event workers: unnecessary and introduces more goroutine lifecycle risk.

### 3) Define `sh_attach` remote-close behavior at the CLI boundary

**Decision:** When the task WebSocket closes or the remote task ends, the interactive CLI must restore terminal state and return without waiting for another stdin byte.

**Implementation direction:**

- Treat the stdin reader as input plumbing, not as a required shutdown blocker for the command.
- Wait only for goroutines that are context-cancellable or otherwise guaranteed to finish.
- Keep best-effort final task-state lookup and `--report` export after WebSocket close, but never allow those steps to reintroduce an interactive hang.
- Use test seams/fakes for stdin/WebSocket so the regression can assert command return while local input is idle.

**Alternatives considered:**

- Close real `os.Stdin`: unsafe for a user terminal and cross-platform fragile.
- Nonblocking terminal polling: more complete in theory but too much platform-specific terminal work for this release.
- Full interactive I/O abstraction: useful later, not required to remove the release blocker.

### 4) Fix direct code defects without changing product design

**Decision:** The remaining findings should be code-only fixes under the existing design.

**Implementation direction:**

- Validate `NatHoleResp` before reading fields from it.
- Build TCP punching targets before starting workers, or otherwise guarantee `jobCh` is closed/cancelled on every early return.
- Make coordinator response send errors visible and stop treating failed sends as successful delivery.
- Make SID generation fail closed if cryptographic random ID generation fails; do not fall back to timestamp-only IDs.
- Close candidate TCP connections when TLS configuration fails before handshakes start.
- Run `gofmt` on the drifted Go files.

## Risks / Trade-offs

- [Dispatcher locking introduces callback deadlock risk] → Copy handler under lock and invoke outside the lock.
- [Async `Send` still cannot prove delivery] → Document that `Send` means accepted; use terminal `Err()`/`Done()` for write-loop failure observation.
- [Changing emitter methods to return errors touches a public package] → Existing ignored call statements remain valid; tests cover checked error behavior.
- [CLI stdin reader may remain blocked until process exit] → Do not let it block command return; keep a future full terminal abstraction out of this release fix.
- [Release fix grows too wide] → Keep changes bound to reviewed findings and focused regression tests.

## Migration Plan

- No data, wire, or config migration is required.
- Existing event callers continue compiling when ignoring returned errors.
- Existing dispatcher callers keep using `Send`/`Done`; callers that need diagnostics can additionally read terminal error state.
- Rollback is a normal code rollback; no persisted state changes are introduced.
