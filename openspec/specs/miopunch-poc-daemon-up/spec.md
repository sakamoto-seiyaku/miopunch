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
The system SHALL enforce that at most one `miopunch up` instance runs per operator on the same machine.
On startup, `miopunch up` SHALL probe both the system and user default LocalAPI addresses.
If either address is reachable, `miopunch up` SHALL exit without starting a second instance and SHALL return actionable diagnostics.

#### Scenario: Starting a second `miopunch up` fails when LocalAPI is already reachable
- **WHEN** a `miopunch up` instance is already running and its LocalAPI is reachable
- **AND** a user runs `miopunch up` again
- **THEN** the second invocation exits without starting a new daemon
- **AND** it provides actionable output indicating an existing daemon is running

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

