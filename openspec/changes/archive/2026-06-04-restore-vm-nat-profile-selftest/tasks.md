## 1. OpenSpec

- [x] 1.1 Create change artifacts for restoring the VM NAT profile selftest.
- [x] 1.2 Validate the OpenSpec change and all active specs.

## 2. Lab Commands

- [x] 2.1 Rename the guest runner from `mlab-selftest` to `mlab-nat-profile-selftest`.
- [x] 2.2 Replace the host `selftest` entrypoint with `nat-profile-selftest`.
- [x] 2.3 Confirm `labctl --help` exposes `nat-profile-selftest` and not `selftest`.

## 3. Guidance And Gates

- [x] 3.1 Update developer guidance to define current required checks as host checks plus `nat-profile-selftest`.
- [x] 3.2 Update `run_test_gates.sh` to run only current required host checks plus `nat-profile-selftest`.
- [x] 3.3 Update lab documentation to describe `nat-profile-selftest` as the restored P0 NAT profile baseline and keep other suites historical/debug-only.

## 4. Validation

- [x] 4.1 Run `openspec validate --all --strict`.
- [x] 4.2 Run host checks: `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [x] 4.3 Run `./lab/host/labctl nat-profile-selftest` or record the exact environment blocker.
- [x] 4.4 Confirm `./lab/host/labctl selftest` is rejected.
