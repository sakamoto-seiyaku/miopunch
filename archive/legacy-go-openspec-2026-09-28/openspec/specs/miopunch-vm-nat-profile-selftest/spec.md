# miopunch-vm-nat-profile-selftest Specification

## Purpose
Defines the current VM/netns NAT profile substrate validation gate for POC v1. This gate restores the old P0 `core-01..core-10` NAT profile baseline under the explicit `nat-profile-selftest` name, without reviving historical XTCP/MNT/POC e2e suites as required validation.

## Requirements

### Requirement: VM NAT profile selftest runs baseline core cases
The system SHALL provide a host lab command named `nat-profile-selftest` that starts or reuses the QEMU lab VM, waits for guest readiness, pushes the guest lab runtime, runs the baseline NAT profile cases `core-01` through `core-10`, and pulls artifacts to `lab/_artifacts/`.

The guest runner SHALL execute only case names matching `core-[0-9]{2}` by default and SHALL fail the command when any selected case fails.

#### Scenario: Baseline NAT profile cases pass
- **WHEN** a developer runs `./lab/host/labctl nat-profile-selftest` on a host with the lab prerequisites available
- **THEN** the guest runner executes `core-01` through `core-10`
- **AND** the summary reports `pass=10 fail=0`
- **AND** artifacts are available under `lab/_artifacts/`

### Requirement: Generic selftest command is not a current entrypoint
The host lab command dispatcher SHALL NOT expose `selftest` as a current command or alias for `nat-profile-selftest`.

#### Scenario: Old selftest name is rejected
- **WHEN** a developer runs `./lab/host/labctl selftest`
- **THEN** the command fails with an unknown-command error
- **AND** the help text points developers at named lab commands including `nat-profile-selftest`

### Requirement: Historical VM suites are not required current gates
The current developer gate automation SHALL run host checks plus `./lab/host/labctl nat-profile-selftest` and SHALL NOT run historical `xtcp-*`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`, or `mnt03-*` lab suites as required current validation.

#### Scenario: Current gate script excludes historical suites
- **WHEN** a developer runs `.codex/skills/dev/scripts/run_test_gates.sh`
- **THEN** the script runs `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`, and `./lab/host/labctl nat-profile-selftest`
- **AND** it does not run `xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`, or `mnt03-*`
