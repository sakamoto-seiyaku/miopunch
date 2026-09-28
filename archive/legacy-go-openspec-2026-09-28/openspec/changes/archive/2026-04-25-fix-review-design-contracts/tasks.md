## 1. Design-Contract Fixes (Docs + Code)

- [x] 1.1 Re-read `docs/notes/2026-04-25-review-fix-design-plan.md`, this change design, and delta specs; update the note if implementation chooses a stronger final approach.
- [x] 1.2 Implement `wire.Dispatcher` as runtime-safe: synchronized handler registry, handler invocation outside locks, idempotent shutdown, stored terminal error, and `Err()`-style observation.
- [x] 1.3 Preserve dispatcher `Send` as async acceptance; return the terminal error when already done and stop discarding `WriteMsg` failures in `sendLoop`.
- [x] 1.4 Add dispatcher tests for post-`Run()` handler registration, write-loop failure closing `Done()`, and observable terminal error.
- [x] 1.5 Change `event.Emitter` methods to return encode/write errors while keeping existing ignored-call behavior valid.
- [x] 1.6 Add event emitter tests with a failing writer and confirm best-effort ignored calls do not panic.
- [x] 1.7 Implement `sh_attach` interactive CLI remote-close behavior so WebSocket close cancels output, restores terminal, and returns without waiting for idle stdin.
- [x] 1.8 Add a focused `sh_attach` regression test using fake/controllable IO or WebSocket seams to prove remote close while stdin is idle does not hang.

## 2. Recommended Code-Only Fixes

- [x] 2.1 Move `NatHoleResp` nil validation before any response field access in attempt setup and add/adjust regression coverage.
- [x] 2.2 Fix TCP punching worker lifetime by building targets before starting workers or by guaranteeing job channel closure/cancellation on every early return.
- [x] 2.3 Handle coordinator visitor/client response send errors and avoid treating failed sends as successful delivery.
- [x] 2.4 Make NAT-hole SID generation fail closed when `authutil.RandID` fails; do not produce timestamp-only SIDs.
- [x] 2.5 Close owned TCP candidate connections when pinned TLS configuration fails before handshakes start.
- [x] 2.6 Run `gofmt` on the drifted Go files reported by `review-current-go-code`.
- [x] 2.7 Fix validation-discovered TCP spraying blocker with bounded TCP punching retries, sender/receiver delay alignment, and the lab `nat4-irregular` DNAT promised by its profile comments.

## 3. Verification

- [x] 3.1 Run focused tests for changed packages, including `go test -race ./internal/wire` if race tooling is available.
- [x] 3.2 Run full host gates: `export PATH=/usr/local/go/bin:$PATH`, `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`.
- [x] 3.3 Run lab gates if the final implementation changes lab/runtime behavior beyond the reviewed CLI/control/data paths.
- [x] 3.4 Confirm `openspec validate fix-review-design-contracts` passes.
