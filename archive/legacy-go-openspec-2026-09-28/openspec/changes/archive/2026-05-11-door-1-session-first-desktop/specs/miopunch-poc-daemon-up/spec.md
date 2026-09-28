## MODIFIED Requirements

### Requirement: Only one daemon instance runs per operator on a machine
The system SHALL enforce that at most one usable `miopunch up` instance runs per operator on the same machine.

On startup, `miopunch up` SHALL probe the default LocalAPI addresses that are relevant to the requested listen mode.

If a same-operator LocalAPI address is reachable, `miopunch up` SHALL exit without starting a second instance and SHALL return actionable diagnostics.

If a privileged/system LocalAPI address is inaccessible to the current non-privileged user, that permission failure SHALL NOT block starting a same-user foreground/session daemon.

#### Scenario: Starting a second `miopunch up` fails when LocalAPI is already reachable
- **WHEN** a `miopunch up` instance is already running and its LocalAPI is reachable
- **AND** a user runs `miopunch up` again
- **THEN** the second invocation exits without starting a new daemon
- **AND** it provides actionable output indicating an existing daemon is running

#### Scenario: Inaccessible system LocalAPI does not block user daemon startup
- **GIVEN** system LocalAPI exists but the current user cannot access it
- **AND** no same-user LocalAPI is reachable
- **WHEN** the user runs `miopunch up` as a non-privileged user
- **THEN** the daemon starts as a same-user foreground/session daemon
- **AND** LocalAPI becomes reachable through the same-user endpoint

## ADDED Requirements

### Requirement: `miopunch up` supports desktop-managed session bootstrap
The system SHALL allow `miopunch up` to be started by `miopunch-desktop` as a same-user session daemon without administrator/root privileges.

The session daemon SHALL serve LocalAPI over the same-user IPC endpoint and SHALL remain compatible with existing LocalAPI task, event, report, and `sh_attach` behavior.

#### Scenario: Desktop starts a same-user daemon
- **WHEN** `miopunch-desktop` starts `miopunch up` for the current user session
- **THEN** `miopunch up` starts without requiring administrator/root privileges
- **AND** the same-user LocalAPI endpoint becomes reachable

#### Scenario: Session daemon preserves existing LocalAPI behavior
- **GIVEN** `miopunch up` was started as a desktop-managed session daemon
- **WHEN** the GUI calls LocalAPI task, event, report, or `sh_attach` operations
- **THEN** the operations use the existing LocalAPI contracts
