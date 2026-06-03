# miopunch-poc-daemon-up Specification

## Purpose
TBD - created by archiving change poc-05-daemon-up-localapi. Update Purpose after archive.
## Requirements
### Requirement: `miopunch up` runs as a foreground daemon and serves LocalAPI v0
The system SHALL provide a `miopunch up` command that starts a foreground daemon process.
While running, the daemon SHALL serve LocalAPI v0 over OS-native IPC (Linux unix socket / Windows named pipe).

#### Scenario: User starts `miopunch up` and LocalAPI becomes reachable
- **WHEN** a user runs `miopunch up`
- **THEN** the daemon starts and continues running in the foreground until terminated
- **AND** `GET /api/v0/status` is reachable via the default LocalAPI address

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

### Requirement: Stale LocalAPI socket/pipe is cleaned up on startup
If the default LocalAPI address points to an existing socket/pipe that is NOT reachable, `miopunch up` SHALL treat it as stale.
The daemon SHALL remove the stale socket/pipe and recreate a fresh listener at the same address.

#### Scenario: A stale unix socket file does not block daemon startup
- **WHEN** the LocalAPI unix socket path exists but no server is reachable via that socket
- **AND** a user runs `miopunch up`
- **THEN** the daemon removes the stale socket file and starts serving LocalAPI at that path

### Requirement: System service installation uses a stable binary path and does not delete state on uninstall
The system SHALL provide:
- `miopunch install-system-daemon` to install and start a system service that runs `miopunch up`
- `miopunch uninstall-system-daemon` to stop and remove that system service

`install-system-daemon` SHALL copy the `miopunch` binary to a stable path before registering the service.
`uninstall-system-daemon` SHALL NOT delete the state directory.

#### Scenario: install copies to a stable path and starts the service
- **WHEN** a user runs `miopunch install-system-daemon`
- **THEN** a system service is installed and started
- **AND** the service executes `miopunch up` from the stable binary path

#### Scenario: uninstall removes the service but preserves state
- **WHEN** a user runs `miopunch uninstall-system-daemon`
- **THEN** the system service is removed
- **AND** the state directory remains intact

### Requirement: `reset` clears state only when the daemon is not running
The system SHALL provide `miopunch reset` to remove the effective state directory and reset the node identity.
`reset` SHALL refuse to run when a LocalAPI daemon is reachable, and SHALL return actionable diagnostics.

#### Scenario: reset refuses to run while daemon is running
- **WHEN** `miopunch up` is running and LocalAPI is reachable
- **AND** a user runs `miopunch reset`
- **THEN** `reset` fails without deleting state
- **AND** it provides actionable output indicating the daemon must be stopped first
