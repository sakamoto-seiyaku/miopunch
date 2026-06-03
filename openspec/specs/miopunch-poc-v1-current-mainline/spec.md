# miopunch-poc-v1-current-mainline Specification

## Purpose
Defines current POC v1 as the active miopunch mainline: shared daemon, LocalAPI, CLI, desktop GUI, Android control-lite, UDP-only direct-first path establishment, secure session, remote shell, and the active validation boundary.

## Requirements

### Requirement: Current active mainline is POC v1
The system SHALL treat current POC v1 as the active mainline capability set.

Current POC v1 SHALL include the shared daemon, LocalAPI, CLI commands, desktop GUI, Android control-lite, POC v1 runtime modules, UDP direct-first path establishment, UDP punching fallback, secure session upgrade, and remote shell demo flow.

Historical P0/P1/P2/MNT/XTCP/TCP Door-2 and POC v0 specs SHALL NOT be active current-mainline requirements after this change.

#### Scenario: Active specs point to POC v1
- **WHEN** a developer inspects `openspec/specs`
- **THEN** the active current-mainline product constraints are expressed through POC v1 specs
- **AND** old MNT, XTCP, TCP Door-2, and POC v0 specs are not present as active current gates

### Requirement: Current POC v1 validation uses host checks and real demo evidence
The current POC v1 validation gate SHALL use host checks plus real Android/Linux/GUI demo evidence.

The host checks SHALL include:

- `go test ./...`
- `go vet ./...`
- `bash scripts/check_no_xtcp_imports.sh`
- `openspec validate --all --strict`

Real demo evidence SHALL cover the current POC v1 flow with logs or reports for network creation or join, peer discovery, `ping`, `sh ls`, and interactive shell over a selected UDP path.

VM lab gates SHALL remain historical/debug-only until a future POC v1 lab capability redefines them.

#### Scenario: VM lab gates are not required for current POC v1 validation
- **WHEN** a docs-only or OpenSpec-only POC v1 mainline realignment is verified
- **THEN** validation does not require `labctl selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, or `xtcp-fulltest`
- **AND** current validation records host checks and real-demo evidence instead

### Requirement: Current POC v1 pathing is UDP-only direct-first
Current POC v1 peer session establishment SHALL use UDP carrier semantics only.

The system SHALL attempt UDP direct reachability before UDP punching fallback when direct candidates are available.

The system SHALL expose selected path evidence that distinguishes at least `direct_ipv4`, `direct_ipv6`, and `punching_ipv4`.

The system SHALL reject `tcp_only` as unsupported in current POC v1 instead of silently falling back to UDP.

#### Scenario: TCP-only policy is explicit unsupported scope
- **WHEN** a current POC v1 peer command requests `p2p_network=tcp_only`
- **THEN** the command fails with an explicit unsupported-path result
- **AND** the runtime does not silently run UDP path establishment as a fallback

### Requirement: Historical specs remain available outside active specs
Historical specs moved out of active OpenSpec SHALL remain available under repository archive with an index explaining why they are no longer active.

The archive SHALL preserve enough original spec content for future reference, comparison, and redesign.

#### Scenario: Historical spec can be found after migration
- **WHEN** a developer needs the old XTCP, MNT, TCP Door-2, POC v0, or lab spec text
- **THEN** the archived spec copy is available under `archive/openspec-specs/2026-06-04-pre-pocv1/`
- **AND** the active `openspec/specs` tree does not treat that text as current
