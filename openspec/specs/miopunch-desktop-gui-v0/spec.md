# miopunch-desktop-gui-v0 Specification

## Purpose
TBD - created by archiving change client-win-linux. Update Purpose after archive.
## Requirements
### Requirement: Desktop GUI connects to daemon via LocalAPI-only with deterministic selection
The system SHALL provide a desktop GUI (`miopunch-desktop`) that connects to the local daemon **only** via `LocalAPI` (IPC: unix socket / Windows named pipe).

By default, the GUI SHALL probe and select a LocalAPI endpoint in this order:
1) system LocalAPI
2) user LocalAPI

The GUI SHALL provide an optional LocalAPI address override (advanced setting) whose semantics are equivalent to CLI `--localapi` and which bypasses the default probe order.

#### Scenario: GUI prefers system LocalAPI when reachable
- **GIVEN** both system and user LocalAPI addresses are known
- **AND** system LocalAPI is reachable
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI connects to system LocalAPI
- **AND** the UI indicates which LocalAPI endpoint is currently in use

#### Scenario: GUI falls back to user LocalAPI when system LocalAPI is not reachable
- **GIVEN** both system and user LocalAPI addresses are known
- **AND** system LocalAPI is not reachable
- **AND** user LocalAPI is reachable
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI connects to user LocalAPI
- **AND** the UI indicates which LocalAPI endpoint is currently in use

#### Scenario: GUI uses override address when configured
- **GIVEN** the user has configured a LocalAPI override address
- **WHEN** the user opens the desktop GUI
- **THEN** the GUI connects to the override LocalAPI address
- **AND** the UI indicates that override mode is enabled

### Requirement: GUI classifies LocalAPI connection failures and provides actionable next steps
When the GUI fails to connect to LocalAPI, it SHALL classify the failure and provide actionable next steps.

At minimum, the GUI SHALL distinguish:
- `bad_request` (invalid override address format)
- `forbidden` (permission denied)
- `daemon_not_running` (endpoint not reachable)
- unknown/other (endpoint reachable but incompatible/unexpected response)

The GUI SHALL show a short summary and 2–3 suggested next actions by default, and SHALL allow expanding details that include: `stage`, `reason_code`, `facts`, and the selected LocalAPI address.

#### Scenario: Permission denied shows operator guidance
- **WHEN** the GUI fails to connect to system LocalAPI due to permission denied
- **THEN** the GUI shows `reason_code=forbidden`
- **AND** the default view includes suggested actions to fix operator permissions (e.g., join `miopunch-operators` on Linux)

#### Scenario: Daemon not running suggests how to start it
- **WHEN** the GUI cannot reach any default LocalAPI endpoint
- **THEN** the GUI shows `reason_code=daemon_not_running`
- **AND** the default view suggests starting the daemon (e.g., `miopunch up`) or installing the system service (e.g., `miopunch install-system-daemon`)

#### Scenario: Unknown/incompatible endpoint suggests repair and log export
- **WHEN** the GUI reaches an endpoint but receives an unexpected response
- **THEN** the GUI indicates the failure may be caused by version mismatch or environment issues
- **AND** the default view suggests using installer Repair/reinstall and exporting logs for diagnosis

### Requirement: GUI renders snapshot-first state and stays updated in near real-time
The GUI SHALL display the daemon state including `status`, `peers`, and `tasks`.

The GUI SHALL keep the view updated in near real-time by consuming the LocalAPI global event stream and applying task events to the UI state.

#### Scenario: UI shows initial snapshot then updates on task events
- **GIVEN** LocalAPI `GET /api/v0/events` is available
- **WHEN** the GUI opens the main screen
- **THEN** the UI renders an initial snapshot of current tasks
- **AND** when a new task event arrives the UI updates the task card/state without requiring manual refresh

### Requirement: GUI supports starting core tasks and viewing task reports
The GUI SHALL allow the user to start core tasks that are already supported by the daemon task system, and SHALL display task progress and final result.

The GUI SHALL allow viewing the task report for a completed task.

#### Scenario: User starts an invite task and observes progress
- **WHEN** the user triggers an invite flow in the GUI
- **THEN** the GUI creates a task via LocalAPI
- **AND** the task appears in the task list and progresses as events arrive

#### Scenario: User views a task report
- **GIVEN** a task has completed
- **WHEN** the user opens the task details in the GUI
- **THEN** the GUI displays the task report content

### Requirement: GUI provides an embedded interactive shell using sh_attach
The GUI SHALL provide an embedded interactive terminal for `sh_attach`.

The system SHALL use the existing LocalAPI WebSocket `sh_attach` contract and SHALL negotiate subprotocol `miopunch.sh.v0`.

The GUI SHALL support terminal window resize by forwarding the appropriate control frames so that the remote side can adjust the PTY window size.

#### Scenario: User opens an interactive shell and receives output
- **GIVEN** the daemon can create a `sh_attach` task to a reachable peer
- **WHEN** the user opens the embedded terminal in the GUI
- **THEN** the GUI establishes a WebSocket attach using subprotocol `miopunch.sh.v0`
- **AND** terminal output is rendered in the UI

#### Scenario: Terminal resize is applied
- **GIVEN** an interactive shell session is open in the GUI
- **WHEN** the user resizes the terminal window
- **THEN** the GUI forwards a resize control message to the attach channel

### Requirement: GUI exports task reports to a user-selected file path
The GUI SHALL allow exporting a task report to a user-selected file path.

The export operation SHALL be performed by the Go bridge (not browser download semantics) so that it works consistently across platforms.

#### Scenario: Report export writes a markdown file to the selected path
- **GIVEN** a completed task has a report available
- **WHEN** the user clicks “Export report” and selects an output path
- **THEN** the system writes the report content to that path
- **AND** the GUI confirms success and shows the output path

