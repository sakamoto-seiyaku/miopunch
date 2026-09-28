## 1. Package Layout Migration

- [x] 1.1 Create the target directory skeleton (`connectivity/`, `event/`, `nat/`, `stun/`, `internal/**`) per the design mapping.
- [x] 1.2 Migrate `xtcp/obs` → `event/` and update all imports without changing event semantics/output format.
- [x] 1.3 Checkpoint: run `go test ./...` + `go vet ./...` (ensure the repo still compiles after moving `event/`).
- [x] 1.4 Migrate `xtcp/connectivity` → `connectivity/` and update all imports.
- [x] 1.5 Checkpoint: run `go test ./...` + `go vet ./...` (ensure `connectivity/` move did not drift behavior or break imports).
- [x] 1.6 Migrate `xtcp/stun` (server) → `stun/` and update all imports.
- [x] 1.7 Checkpoint: run `go test ./...` + `go vet ./...` (ensure `stun/` move is safe).
- [x] 1.8 Migrate glue/implementation packages into `internal/` (`xtcp/control`, `xtcp/coord`, `xtcp/msg`, `xtcp/transport`, `xtcp/netutil`, `xtcp/peer`) and update imports.
- [x] 1.9 Checkpoint: run `go test ./...` + `go vet ./...` (post-`internal/` migration).
- [x] 1.10 Checkpoint: run `./lab/host/labctl xtcp-selftest` and confirm artifacts validate (P1 regression).
- [x] 1.11 Checkpoint: run `./lab/host/labctl xtcp-connectivity-selftest` and confirm artifacts validate (P2 regression).

## 2. Nathole + Util Cleanup (Minimal, No Behavior Drift)

- [x] 2.1 Split `xtcp/nathole` into `nat/` + `internal/punching/` + `internal/coordinator/` (minimal move-first; avoid deep refactors).
- [x] 2.2 Checkpoint: run `go test ./...` + `go vet ./...` (post-`nathole` split).
- [x] 2.3 Remove `xtcp/util/**` buckets by moving code into ownership-aligned locations (`internal/*util/` or owner packages) and update call sites.
- [x] 2.4 Checkpoint: run `go test ./...` + `go vet ./...` (post-`util` cleanup).
- [x] 2.5 Ensure there is no top-level `xtcp/` directory remaining after migration.
- [x] 2.6 Checkpoint: verify no `github.com/miopunch/miopunch/xtcp` imports remain (e.g. `rg -n -- \"github.com/miopunch/miopunch/xtcp\"`) and rerun `go test ./...`.
- [x] 2.7 Checkpoint: rerun `./lab/host/labctl xtcp-selftest` and `./lab/host/labctl xtcp-connectivity-selftest` (prove P0/P1/P2 experiment entry points still work after the big split).

## 3. CLI, Lab, and Docs Alignment

- [x] 3.1 Update `cmd/miopunch` imports to the new package layout and remove `xtcp` from user-facing help/usage text.
- [x] 3.2 Checkpoint: run `go test ./...` and verify `miopunch help` output contains no `xtcp`.
- [x] 3.3 Update `lab/**` scripts and validators (if any) that hard-code old paths/terms so the regression runner still works.
- [x] 3.4 Checkpoint: run `./lab/check.sh` (syntax + guest unit + openspec validate all).
- [x] 3.5 Update `docs/roadmap.md` to reference current archived change paths and to use `miopunch` naming for `P3` (do not rewrite historical reports).
- [x] 3.6 Checkpoint: run `openspec validate --all --strict --no-interactive`.

## 4. Verification / Guardrails

- [x] 4.1 Add a guardrail check (script + documented command, and/or wired into `lab/check.sh`) that fails if any new `github.com/miopunch/miopunch/xtcp` imports are introduced.
- [x] 4.2 Final checkpoint: run `go test ./...` + `go vet ./...` from the final migrated tree.
- [x] 4.3 Final checkpoint: run `./lab/host/labctl xtcp-selftest` and `./lab/host/labctl xtcp-connectivity-selftest`.
