# miopunch-poc-v1-persistence Specification

## Purpose
Defines the POC v1 on-disk state layout and persistence rules.

## ADDED Requirements

### Requirement: State layout is device/ + networks/<network_id>/
The system SHALL store device-global keys under `device/` and network-scoped state under `networks/<network_id>/`.

#### Scenario: Device and network state land in separate roots
- **WHEN** a v1 node persists long-lived state for one or more joined networks
- **THEN** device-global keys are stored under `device/`
- **AND** each network's state is stored under `networks/<network_id>/`

### Requirement: State writes are atomic
The system SHALL write state files atomically (tmp + rename) to avoid partial writes.

#### Scenario: Interrupted write does not leave a partial state file
- **WHEN** a v1 state file is rewritten
- **THEN** the runtime writes a temporary file and promotes it atomically
- **AND** readers never observe a partially written final file

### Requirement: State file permissions are restrictive
The system SHALL ensure state directories and files are created with restrictive permissions (0700/0600).

#### Scenario: Persisted secrets are written with locked-down permissions
- **WHEN** the runtime creates a v1 state directory or file
- **THEN** directories use `0700` permissions
- **AND** files use `0600` permissions
