---
name: dev
description: "Miopunch development playbook and guardrails. Use for any miopunch repo work: implementing features/fixes, refactors, adding tests, updating specs/docs, creating or applying OpenSpec changes, and running lab verification. Enforces naming/layering rules plus required test gates (go test/vet + labctl selftests/fulltest) before commit/archive."
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
   - Run the required test gates (below)
5. Archive:
   - Use `$openspec-archive-change` only after all gates pass

## Guardrails

- **Naming**: prefer `miopunch` everywhere; avoid new `xtcp` names/paths/imports.
- **Layering**:
  - `connectivity/`: outward-facing semantics
  - `internal/punching/`: low-level punching implementation
  - `dataplane/` + `internal/dataplane/`: payload transports (kcp/quic/brutal, etc.)
  - Punching outputs a usable UDP path; transports own “payload exchange”.
- **Scope**: do not over-optimize or over-design; prioritize runnable, observable connectivity.
- **Tests**: do not “fix” baseline NAT/P0 scenarios by mutating them; add new cases for new behavior.

## Required Test Gates

### Host checks (always)

Note: in this environment `go` might not be in `PATH`; prefer exporting `/usr/local/go/bin`.

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
```

### Lab checks (VM)

```bash
./lab/host/labctl selftest
./lab/host/labctl xtcp-selftest
./lab/host/labctl xtcp-connectivity-selftest
./lab/host/labctl xtcp-fulltest
```

Artifacts are pulled into `lab/_artifacts/`.

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

- Run the full gate set: `bash .codex/skills/dev/scripts/run_test_gates.sh`
- Reference notes: read `.codex/skills/dev/references/dev.md` when needed
