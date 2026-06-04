---
name: dev
description: "Miopunch development playbook and guardrails. Use for any miopunch repo work: implementing features/fixes, refactors, adding tests, updating specs/docs, creating or applying OpenSpec changes, and running lab verification. Enforces naming/layering rules and requires the full test gates when code-affecting changes are committed or merged into mainline."
---

# Miopunch Dev

Use this skill as the default workflow for development work in this repo.

## Workflow

1. If the request is **exploration / design alignment** (no code yet):
   - Use `$openspec-explore`
   - Write decisions/charter first (then implement)
2. If the request needs a tracked change:
   - Prefer `$openspec-propose` (one-shot proposal) or `$openspec-new-change` (step-by-step)
3. Implement:
   - Use `$openspec-apply-change`
4. Verify:
   - Use `$openspec-verify-change`
   - For code-affecting changes entering mainline, run the required full test gates (below)
5. Archive:
   - Use `$openspec-archive-change` only after the required validation level passes

## Real Environment Debugging

- Use this repo's real Windows/WSL2 environment as the source of truth for mirrored-network, daemon, LocalAPI, and governance-state issues.
- Break connectivity failures into stages before assigning root cause: signaling, candidate gather, punching, dataplane, hello/governance, payload, and session lifecycle.
- Validate in small batches when a failure spans multiple linked problems; keep the batch boundary explicit in notes and commits.
- For live debug batches, collect focused validation and real-environment evidence before committing; do not treat this as replacing required gates unless the user explicitly narrows scope.
- If validation scope is narrowed, do so only because the user explicitly requested it, and record that override in the task notes or change log.

## Go Companion Skills (Router)

When changing Go code in this repo, treat the `go-*` skills as a router. **At
minimum, always apply** `$go-style-core`, `$go-naming`, `$go-error-handling` in
addition to `$dev`, then add the rest based on what the diff touches.

- **Default for most Go changes**: `$go-style-core`, `$go-naming`, `$go-error-handling`
- **Packages / imports / splitting binaries / `cmd/` work**: `$go-packages`
- **Tests**: `$go-testing`
- **Exported APIs / doc comments**: `$go-documentation`
- **Logging changes**: `$go-logging`
- **Performance work**: `$go-performance`
- **Interfaces / mock boundaries**: `$go-interfaces`
- **Generics**: `$go-generics`
- **Many optional settings in a constructor**: `$go-functional-options`
- **Concurrency (required)**: use `$go-concurrency` when goroutines, channels, mutexes,
  WaitGroups, shared state, worker lifecycles, or parallel execution are involved
- **Context (required)**: use `$go-context` when cancellation, deadlines, or timeouts
  must be propagated or enforced

**Quick heuristic (most common in miopunch):**
- Editing `cmd/*`, imports, entrypoints, binary splits → add `$go-packages`
- Touching `context.WithTimeout/Deadline`, timeouts, cancellation flow → add `$go-context`
- Touching goroutines/channels/`errgroup`/shared state → add `$go-concurrency`

**Quick trigger checklist (scan the diff):**
- Any `*_test.go` changes → add `$go-testing`
- Any `context.` usage (timeouts, deadlines, cancellation) → add `$go-context`
- Any `go` statement / `chan` / `sync.` / `atomic.` / `errgroup` / shared state → add `$go-concurrency`
- New package/folder, file moves, or import reshapes → add `$go-packages`
- New exported identifiers (public API surface) → add `$go-documentation`
- Logging (`log.*`, `slog.*`) changes → add `$go-logging`
- Interfaces/mocking boundaries → add `$go-interfaces`
- Perf-sensitive paths / allocation work → add `$go-performance`

## Guardrails

- **Naming**: prefer `miopunch` everywhere; avoid new `xtcp` names/paths/imports.
- **Layering**:
  - `connectivity/`: outward-facing semantics
  - `internal/punching/`: low-level punching implementation
  - `dataplane/` + `internal/dataplane/`: payload transports (kcp/quic/brutal, etc.)
  - Punching outputs a usable UDP path; transports own “payload exchange”.
- **Scope**: do not over-optimize or over-design; prioritize runnable, observable connectivity.
- **Tests**: do not “fix” baseline NAT/P0 scenarios by mutating them; add new cases for new behavior.

## Validation Policy

- **Run the full gate set** when a code-affecting change enters the mainline branch, whether by direct commit or merge.
- **Code-affecting** includes Go code, tests, lab/runtime scripts, and other execution-affecting files.
- **Docs-only / notes-only / OpenSpec-only** changes do not require the full gate set unless explicitly requested.
- If the user explicitly narrows verification for a debug batch, treat that as a user override, not a default policy change.
- For non-mainline iteration work, prefer focused validation that matches the touched code.

## Required Test Gates

### Host checks (full gate set)

Note: in this environment `go` might not be in `PATH`; prefer exporting `/usr/local/go/bin`.

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
```

### Lab checks (VM, full gate set)

```bash
./lab/host/labctl nat-profile-selftest
```

Artifacts are pulled into `lab/_artifacts/`.

Historical/debug suites such as `xtcp-*`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`,
and `mnt03-*` are not current required gates.

Optional cleanup:

```bash
./lab/host/labctl down
```

## Event/Test Expectations

- Add explicit validation for critical events (e.g. “payload exchanged”) via lab `cases/expect/*.events.json`.
- When transport is the focus (not punching), prefer a “known-good punching” baseline case (e.g. `core-01`).

## Troubleshooting (lab)

- If SSH port/config looks stale, regenerate it:

  ```bash
  rm -f lab/_state/ssh_config
  ./lab/host/labctl wait
  ```

## Resources

- Run the full gate set when required by the validation policy: `bash .codex/skills/dev/scripts/run_test_gates.sh`
- Reference notes: read `.codex/skills/dev/references/dev.md` when needed
