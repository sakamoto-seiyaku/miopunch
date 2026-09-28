## 1. Progressive Gate Plumbing

- [x] 1.1 Add progressive checkpoint execution to `mlab-mnt03-run`
- [x] 1.2 Change public `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest` wrappers to call the progressive flow
- [x] 1.3 Preserve existing fresh-start stages as manual/debug entry points

## 2. Checkpoint Evidence

- [x] 2.1 Emit checkpoint-specific topology and proof artifacts for 2, 3, 4, 6, 8, 12, and perturbation checkpoints
- [x] 2.2 Ensure checkpoint failures report the checkpoint name in the stage summary
- [x] 2.3 Keep final aggregate summaries machine-readable and compatible with existing host artifact pull

## 3. Documentation and Specs

- [x] 3.1 Update the MNT-03 charter to describe default progressive growth gates and debug fresh-start stages
- [x] 3.2 Update the main MNT-03 spec to match the progressive public gate contract
- [x] 3.3 Validate the OpenSpec change

## 4. Validation

- [x] 4.1 Run shell syntax checks for changed MNT-03 scripts
- [x] 4.2 Run host checks: `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`
- [x] 4.3 Run MNT-03 smoke/self/full gates or record the blocker with artifact evidence
  - `mnt03-smoke`: pass, `lab/_artifacts/20260430T145321Z-mnt03-smoke-aggregate/summary.json`
  - `mnt03-selftest`: pass, `lab/_artifacts/20260430T150149Z-mnt03-selftest-aggregate/summary.json`
  - `mnt03-fulltest`: pass, `lab/_artifacts/20260430T154355Z-mnt03-fulltest-aggregate/summary.json`
