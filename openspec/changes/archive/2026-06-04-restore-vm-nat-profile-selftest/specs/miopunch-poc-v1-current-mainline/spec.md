## MODIFIED Requirements

### Requirement: Current POC v1 validation uses host checks and real demo evidence
The current POC v1 validation gate SHALL use host checks, the VM NAT profile substrate selftest, and real Android/Linux/GUI demo evidence.

The host checks SHALL include:

- `go test ./...`
- `go vet ./...`
- `bash scripts/check_no_xtcp_imports.sh`
- `openspec validate --all --strict`

The VM NAT profile substrate selftest SHALL be `./lab/host/labctl nat-profile-selftest`.

Real demo evidence SHALL cover the current POC v1 flow with logs or reports for network creation or join, peer discovery, `ping`, `sh ls`, and interactive shell over a selected UDP path.

Historical `xtcp-*`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`, and `mnt03-*` VM lab suites SHALL remain historical/debug-only and SHALL NOT be current required validation gates.

#### Scenario: Current POC v1 validation uses named NAT profile lab gate
- **WHEN** a code-affecting POC v1 mainline change is verified
- **THEN** validation includes the host checks
- **AND** validation includes `./lab/host/labctl nat-profile-selftest`
- **AND** validation does not require `labctl selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`, or `mnt03-*`
