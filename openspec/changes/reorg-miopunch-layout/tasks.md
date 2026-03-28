## 1. Package Layout Migration

- [ ] 1.1 Create the target directory skeleton (`connectivity/`, `event/`, `nat/`, `stun/`, `internal/**`) per the design mapping.
- [ ] 1.2 Migrate `xtcp/obs` → `event/` and update all imports without changing event semantics/output format.
- [ ] 1.3 Migrate `xtcp/connectivity` → `connectivity/` and update all imports.
- [ ] 1.4 Migrate `xtcp/stun` (server) → `stun/` and update all imports.
- [ ] 1.5 Migrate glue/implementation packages into `internal/` (`xtcp/control`, `xtcp/coord`, `xtcp/msg`, `xtcp/transport`, `xtcp/netutil`, `xtcp/peer`) and update imports.

## 2. Nathole + Util Cleanup (Minimal, No Behavior Drift)

- [ ] 2.1 Split `xtcp/nathole` into `nat/` + `internal/punching/` + `internal/coordinator/` (minimal move-first; avoid deep refactors).
- [ ] 2.2 Remove `xtcp/util/**` buckets by moving code into ownership-aligned locations (`internal/*util/` or owner packages) and update call sites.
- [ ] 2.3 Ensure there is no top-level `xtcp/` directory remaining after migration.

## 3. CLI, Lab, and Docs Alignment

- [ ] 3.1 Update `cmd/miopunch` imports to the new package layout and remove `xtcp` from user-facing help/usage text.
- [ ] 3.2 Update `lab/**` scripts and validators (if any) that hard-code old paths/terms so the regression runner still works.
- [ ] 3.3 Update `docs/roadmap.md` to reference current archived change paths and to use `miopunch` naming for `P3` (do not rewrite historical reports).

## 4. Verification / Guardrails

- [ ] 4.1 Run `go test ./...` and fix compilation/tests until green.
- [ ] 4.2 Run `openspec validate --strict --no-interactive` and fix any spec/change validation issues.
- [ ] 4.3 Add a guardrail check (CI/local script or documented command) that fails if any new `github.com/miopunch/miopunch/xtcp` imports are introduced.
- [ ] 4.4 Run a minimal `P0` smoke regression (at least `core-01`) to confirm the entry points still work after the reorg.

