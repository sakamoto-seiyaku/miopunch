## MODIFIED Requirements

### Requirement: Desktop GUI connects to daemon via LocalAPI-only with deterministic selection
The system SHALL provide a desktop GUI (`miopunch-desktop`) that connects to the local daemon **only** via `LocalAPI` (IPC: unix socket / Windows named pipe).

By default, the GUI SHALL use the current session selection order:
1) same-user session LocalAPI
2) same-user session daemon bootstrap
3) already-reachable system LocalAPI, only when it can be used by the current user

The GUI SHALL provide an optional LocalAPI address override (advanced setting) whose semantics are equivalent to CLI `--localapi` and which bypasses the default selection order and daemon bootstrap.

#### Scenario: GUI reuses same-user LocalAPI when reachable
- **GIVEN** the same-user session LocalAPI is reachable
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI connects to that LocalAPI endpoint
- **AND** the UI indicates which LocalAPI endpoint is currently in use

#### Scenario: GUI bootstraps same-user daemon when no usable endpoint is reachable
- **GIVEN** no usable default LocalAPI endpoint is reachable
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI starts a same-user session daemon
- **AND** the GUI connects to the daemon through LocalAPI after it becomes ready
- **AND** the UI indicates that the daemon is managed by the desktop session

#### Scenario: System LocalAPI permission failure does not block session bootstrap
- **GIVEN** system LocalAPI exists but the current user cannot access it
- **AND** same-user session LocalAPI is not reachable
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI starts or connects to a same-user session daemon
- **AND** the system permission failure is available as diagnostic detail, not the primary blocking error

#### Scenario: GUI uses override address when configured
- **GIVEN** the user has configured a LocalAPI override address
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI connects to the override LocalAPI address
- **AND** the UI indicates that override mode is enabled
- **AND** the GUI does not bootstrap a session daemon for that connection attempt

### Requirement: GUI classifies LocalAPI connection failures and provides actionable next steps
When the GUI fails to connect to or bootstrap LocalAPI, it SHALL classify the failure and provide actionable next steps.

At minimum, the GUI SHALL distinguish:
- `bad_request` (invalid override address format)
- `forbidden` (permission denied for the selected explicit endpoint)
- `daemon_not_running` (endpoint not reachable and bootstrap was not attempted)
- `unavailable` (session daemon bootstrap failed or timed out)
- unknown/other (endpoint reachable but incompatible/unexpected response)

The GUI SHALL show a short summary and 2-3 suggested next actions by default, and SHALL allow expanding details that include: `stage`, `reason_code`, `facts`, selected LocalAPI address, and whether a session daemon was bootstrapped.

#### Scenario: Permission denied on system LocalAPI is diagnostic during default startup
- **WHEN** default startup observes permission denied on system LocalAPI
- **AND** the GUI can start or connect to a same-user session daemon
- **THEN** the GUI does not show `reason_code=forbidden` as the blocking state
- **AND** the system permission failure appears only in expanded diagnostics

#### Scenario: Bootstrap failure shows session daemon guidance
- **WHEN** the GUI cannot connect to any usable LocalAPI endpoint
- **AND** starting the same-user session daemon fails or times out
- **THEN** the GUI shows a session bootstrap failure
- **AND** the default view suggests retrying, checking the sibling `miopunch` binary, and exporting runtime diagnostics
- **AND** the default view does not require `miopunch install-system-daemon`

#### Scenario: Unknown/incompatible endpoint suggests version and bundle checks
- **WHEN** the GUI reaches an endpoint but receives an unexpected response
- **THEN** the GUI indicates the failure may be caused by version mismatch or endpoint mismatch
- **AND** the default view suggests checking that `miopunch` and `miopunch-desktop` came from the same bundle and exporting runtime diagnostics

## ADDED Requirements

### Requirement: Desktop GUI owns only desktop-managed daemon shutdown
When `miopunch-desktop` starts a same-user session daemon, it SHALL track that daemon as desktop-managed.

The GUI SHALL stop a desktop-managed daemon on explicit application quit. The GUI SHALL NOT stop a daemon that it merely reused.

#### Scenario: Explicit quit stops desktop-managed daemon
- **GIVEN** the GUI started a same-user session daemon
- **WHEN** the user explicitly quits the desktop application
- **THEN** the GUI best-effort stops the desktop-managed daemon

#### Scenario: Explicit quit preserves reused daemon
- **GIVEN** the GUI connected to a daemon that was already running
- **WHEN** the user explicitly quits the desktop application
- **THEN** the GUI closes without stopping that daemon

### Requirement: Desktop GUI is single-instance with platform window residency semantics
The desktop GUI SHALL enforce a single running GUI instance per user session.

On a second launch, the existing GUI window SHALL be restored and focused instead of starting an independent GUI process.

Windows close semantics SHALL hide the window and keep the application resident. Linux close semantics SHALL prefer tray-backed hide/resident behavior when a reliable tray is available; if no reliable tray is available, closing the window SHALL exit the application.

#### Scenario: Second launch restores existing window
- **GIVEN** `miopunch-desktop` is already running
- **WHEN** the user launches `miopunch-desktop` again
- **THEN** the existing window is shown and focused
- **AND** no second independent GUI instance remains running

#### Scenario: Windows close hides and keeps session resident
- **WHEN** a Windows user closes the desktop window
- **THEN** the window is hidden
- **AND** the desktop session remains resident
- **AND** the user can restore the window from the supported desktop affordance

#### Scenario: Linux without reliable tray exits safely
- **GIVEN** the Linux desktop environment has no reliable tray support
- **WHEN** the user closes the desktop window
- **THEN** the application exits instead of remaining hidden without a recovery affordance
