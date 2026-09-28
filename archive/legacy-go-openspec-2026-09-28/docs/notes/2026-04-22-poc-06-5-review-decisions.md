# POC-06.5 review decisions (2026-04-22)

This note records decisions from review/verification of OpenSpec change
`poc-06-5-governance-report-export`, so we don’t lose them during the upcoming
governance snapshot redesign discussion.

## Decision 1: report persistence errors must be visible (R-A)

### Problem

- Task reports are generated in-memory and exposed via LocalAPI, but persisting
  them under `reports/<task_id>.md` can fail silently.
- Current behavior discards the error:
  - `internal/task/manager.go:437` → `_ = m.persistReport(taskID, report)`

### Root cause

- `persistReport` is treated as best-effort, but there is no user-visible
  surface for persistence failures (permissions, disk full, invalid state_dir,
  etc.), so operators cannot tell the report wasn’t actually written.

### Decision

- Keep task outcome unchanged (do **not** fail the task if report persistence
  fails).
- Make report persistence failures **visible** in the task result (facts /
  suggestions) and (optionally) in the returned report markdown.

### Planned change (implementation sketch)

- In `internal/task/manager.go:436`, handle `err := m.persistReport(...)` instead
  of ignoring it.
- Surface a fact/suggestion on failure, e.g.:
  - fact: `report_persist_failed: <err>`
  - suggestion: check `state_dir` permissions / disk space
- Keep SSE `report_ready` meaning “report content ready” (not necessarily
  persisted).

## Decision 2: unify atomic file write helper (W-A)

### Problem

- Two duplicated `writeFileAtomic` helpers exist with inconsistent durability:
  - `internal/task/report_persist.go:97` syncs the file but not the directory,
    and uses `Remove(path)` + `Rename(tmp, path)`.
  - `cmd/miopunch/report_export.go:43` doesn’t call `Sync()` at all, and also
    uses `Remove(path)` + `Rename(tmp, path)`.
- The repo’s persistence checklist requires “tmp→fsync→rename” atomic updates:
  `docs/notes/2026-04-20-poc-implementation-checklist.md` (section 6).

### Root cause

- Duplicated helper functions and no single, reviewed definition of “atomic
  write” semantics across OSes (especially Windows replace-existing behavior).

### Decision

- Introduce a shared helper for atomic writes and use it from both daemon report
  persistence and CLI `--report` export.
- Target semantics:
  - tmp file in same dir
  - chmod (where applicable)
  - write
  - `fsync(tmp)`
  - close
  - rename/replace-existing
  - `fsync(parent dir)` best-effort on platforms where it applies

### Planned change (implementation sketch)

- New internal package (name TBD): `internal/atomicfile` or `internal/fileutil`.
- Replace:
  - `internal/task/report_persist.go:97` helper
  - `cmd/miopunch/report_export.go:43` helper
- Ensure Windows uses a true “replace existing” primitive (instead of
  remove+rename), while Unix keeps `os.Rename` + dir fsync.

## Open items (not decided here)

- Governance snapshot semantics: treat as a **design problem** and redesign the
  format/validation/apply rules. (Next focus.)
- `gofmt` failures: must fix once we start code changes.

## Governance snapshot redesign (draft decisions; 2026-04-22)

> These are **directional** decisions for the upcoming redesign. Details (esp.
> bootstrap semantics) are still under discussion.

### Documentation workflow

- Update the canonical design draft under `docs/` first, then update OpenSpec
  change artifacts (`openspec/changes/.../design.md` and delta specs).

### Format upgrade

- Upgrade `governance/head_snapshot.json` format directly (no compatibility
  burden assumed; no formal releases yet).
- Adopt a TUF-style signature encoding using `key_id` (instead of a single
  `owner_sig_b64` field).

### Rotation verification (best practice)

- Require both:
  - **old-threshold**: at least one signature verifiable under the local current
    owners (old trust root)
  - **new-threshold**: at least one signature verifiable under the candidate
    snapshot’s owners (self-signed new trust root)

### Genesis semantics

- `prev_hash_b64` for genesis is the empty string `""` (not `"GENESIS"`).

### Snapshot body fields

- Add both `height` and `net_id` back into `snapshot_body`, and validate them
  during apply.
