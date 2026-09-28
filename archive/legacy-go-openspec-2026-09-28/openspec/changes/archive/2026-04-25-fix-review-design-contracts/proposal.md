## Why

The full Go review in `review-current-go-code` found release-blocking design-contract gaps in internal runtime plumbing plus several direct code defects. Because a release is approaching, this change fixes the contract gaps with specs/docs/code and also clears the remaining reviewed code-only issues so the same failure modes are not left behind.

## What Changes

- Clarify and implement release-safe contracts for:
  - `wire.Dispatcher` handler registration, concurrent reads, send semantics, and write-failure observability.
  - `event.Emitter` write failures, keeping best-effort convenience calls while adding a checkable error path for critical diagnostics.
  - interactive `sh_attach` exit behavior when the remote task/WebSocket closes while local stdin is idle.
- Fix direct implementation defects from the review report:
  - nil `NatHoleResp` panic in attempt setup.
  - TCP punching worker lifetime leak when target construction fails.
  - ignored coordinator response send errors.
  - predictable SID fallback when random ID generation fails.
  - leaked TCP candidates on TLS config failure.
  - repository `gofmt` drift.
- Add targeted unit/race-oriented tests for the fixed contracts and regressions.
- Update the temporary design note if the final implementation chooses a more complete release-safe approach than the current recommendation.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-code-health`: add release-quality runtime contracts for internal Go infrastructure: race-free dispatcher handler access, observable async write/event errors, bounded goroutine/resource lifetimes, secure ID generation, and gofmt cleanliness.
- `miopunch-poc-shell-v0`: define interactive `sh_attach` CLI behavior when the WebSocket or remote task ends while local stdin is idle.

## Impact

- Affected code:
  - `internal/wire`, `internal/coordinator`, `event`, `cmd/miopunch`, `connectivity`, `dataplane`, and affected tests.
- Affected docs/specs:
  - `docs/notes/2026-04-25-review-fix-design-plan.md`
  - `openspec/specs/miopunch-code-health/spec.md`
  - `openspec/specs/miopunch-poc-shell-v0/spec.md`
- Public compatibility:
  - No wire-format changes.
  - No new `xtcp` naming, paths, or imports.
  - Existing best-effort event convenience calls remain available; a checkable event error path is added for critical callers.
- Validation:
  - Focused unit tests for changed packages during implementation.
  - Full host gates before mainline: `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`.
  - Lab gates if implementation changes lab/runtime behavior beyond the reviewed CLI/control/data paths.
