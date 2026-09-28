## 1. OpenSpec Artifacts

- [x] 1.1 Create proposal, design, delta specs, and tasks for `realign-mainline-specs-to-pocv1`.
- [x] 1.2 Validate the change artifacts before applying the migration.

## 2. Active Spec Migration

- [x] 2.1 Copy historical active specs into `archive/openspec-specs/2026-06-04-pre-pocv1/` with an archive index.
- [x] 2.2 Remove historical P0/P1/P2/MNT/XTCP/TCP Door-2 and POC v0 capabilities from active `openspec/specs`.
- [x] 2.3 Add `miopunch-poc-v1-current-mainline` to active specs.
- [x] 2.4 Sync completed POC v1 Android, GUI, desktop shutdown, UDP owner/session, and direct Android/WSL changes into main specs.
- [x] 2.5 Update `miopunch-public-reachability` and `miopunch-release-automation-v0` to the current POC v1 gate.

## 3. Context and Runbook Alignment

- [x] 3.1 Update `openspec/project.md` to describe current POC v1 as the active mainline.
- [x] 3.2 Update `docs/roadmap.md` so old P0/P1/P2/MNT/XTCP/TCP Door-2 work is historical or deferred.
- [x] 3.3 Update `lab/README.md` so legacy VM lab commands are historical/debug, not current POC v1 gates.
- [x] 3.4 Keep `AGENTS.md` validation guidance aligned with the current POC v1 gate.

## 4. Verification and Archive

- [x] 4.1 Run `openspec validate --all --strict`.
- [x] 4.2 Run host checks: `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [x] 4.3 Archive completed POC v1/demo changes whose specs have been synced into main specs.
- [x] 4.4 Archive `realign-mainline-specs-to-pocv1` after all tasks are complete.
