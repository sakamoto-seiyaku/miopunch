## 1. Decision Module Extraction

- [x] 1.1 Create `internal/punchdecision` as the neutral home for NAT/punching decision logic.
- [x] 1.2 Move analyzer scoring, STUN view arbitration, response derivation, TCP `+100` derivation, and helper tests from `internal/coordinator` into `internal/punchdecision`.
- [x] 1.3 Expose a minimal decision API that accepts visitor/client `wire` snapshots and returns visitor/client `NatHoleResp` outputs.
- [x] 1.4 Preserve stateful analyzer behavior needed by lab coord success reports and cleanup.

## 2. Callers And Adapters

- [x] 2.1 Update MQTT visitor leader code to call `internal/punchdecision` instead of `internal/coordinator`.
- [x] 2.2 Update POC dialing/task code to call `internal/punchdecision` instead of `internal/coordinator`.
- [x] 2.3 Update `internal/coordinator` to act as the lab coord service adapter over `internal/punchdecision`.
- [x] 2.4 Keep `miopunch-lab coord`, `--coord`, lab YAML `coord`, and lab scripts behavior compatible.

## 3. Specs And Documentation

- [x] 3.1 Update main specs to use neutral “punching decision boundary/engine” wording for MQTT and TCP derivation requirements.
- [x] 3.2 Update product-facing docs/notes that describe current or future MQTT/backend exchange so they no longer imply a product `coord` service dependency.
- [x] 3.3 Leave lab-only docs and historical notes free to mention `coord` when referring to `miopunch-lab coord` or archived history.

## 4. Tests And Validation

- [x] 4.1 Run focused tests for `internal/punchdecision`, `internal/coordinator`, `internal/peer`, and `internal/task`.
- [x] 4.2 Verify product/POC paths no longer import `github.com/miopunch/miopunch/internal/coordinator`.
- [x] 4.3 Run `openspec validate remove-coord-service-semantics`.
- [x] 4.4 For the future code-affecting apply, run host gates: `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
