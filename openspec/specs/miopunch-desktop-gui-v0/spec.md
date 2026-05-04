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

### Requirement: Desktop task actions recover from bridge failures
The desktop GUI SHALL make task-starting controls recoverable when the Wails bridge call fails or does not settle.

#### Scenario: Invite creation bridge failure is visible and recoverable
- **WHEN** the user triggers the invite Create action and the bridge returns an error
- **THEN** the GUI shows a visible failure message
- **AND** the Create action becomes available again

#### Scenario: Invite creation bridge timeout is visible and recoverable
- **WHEN** the user triggers the invite Create action and the bridge call does not settle within the UI timeout
- **THEN** the GUI shows a visible timeout failure message
- **AND** the Create action becomes available again

### Requirement: Desktop invite creation has browser smoke coverage
The desktop GUI SHALL include automated browser-level smoke coverage for the Access invite creation click flow.

#### Scenario: Invite creation click flow renders the invite code
- **WHEN** the browser smoke test opens Access, selects Invite, and clicks Create with a successful fake bridge response
- **THEN** the test observes the bridge call for an invite task
- **AND** the invite code is rendered in the GUI

### Requirement: Desktop GUI has automated browser coverage for primary navigation
The desktop GUI SHALL include CI-run browser tests that verify the committed static UI can navigate between primary tabs and their second-level desktop views without JavaScript errors.

#### Scenario: Primary tabs render their overview pages
- **WHEN** the browser test opens the desktop UI with a fake owner bridge and selects Network, Access, Admin, and Settings
- **THEN** each tab renders its overview content
- **AND** no browser page error or unexpected console error is emitted

#### Scenario: Second-level views use the desktop interaction model
- **WHEN** the browser test opens a peer, access flow, member detail, or settings section
- **THEN** the selected detail view renders within the current primary tab
- **AND** returning to Overview restores the primary tab overview

### Requirement: Desktop GUI has automated browser coverage for role-gated controls
The desktop GUI SHALL include CI-run browser tests that verify owner/admin-only controls and unsafe member operations are hidden or disabled for users that cannot run them.

#### Scenario: Member role cannot access admin-only desktop controls
- **WHEN** the browser test opens the desktop UI with a fake member bridge
- **THEN** admin-only primary navigation and Access flows are unavailable

#### Scenario: Unsafe peer and member actions expose correct disabled states
- **WHEN** the browser test opens self, revoked, owner, admin, disconnected, and remote member states
- **THEN** actions that are not valid for that state are disabled or unavailable

### Requirement: Desktop GUI has automated browser coverage for bridge action calls
The desktop GUI SHALL include CI-run browser tests that verify user actions call the Wails bridge with the expected task kind and object arguments.

#### Scenario: Access actions create expected tasks
- **WHEN** the browser test submits Join, Invite, and Approve flows with valid input
- **THEN** the fake bridge records `join`, `invite`, and `approve` task calls with object arguments
- **AND** the UI renders the task result or progress state

#### Scenario: Network and Admin actions create expected tasks
- **WHEN** the browser test triggers peer Ping, peer List sessions, Shell attach, and member Revoke where enabled
- **THEN** the fake bridge records the expected task kinds and peer arguments
- **AND** the UI renders a visible task or shell state

### Requirement: Desktop GUI has automated browser coverage for runtime updates and recoverable failures
The desktop GUI SHALL include CI-run browser tests that verify runtime events and bridge failures update the UI predictably.

#### Scenario: Runtime task events update rendered task state
- **WHEN** the fake runtime emits task snapshot, stage, fact, and done events
- **THEN** the GUI updates visible task state without a manual refresh

#### Scenario: Bridge failures remain visible and recoverable
- **WHEN** the fake bridge returns an error or never settles for a tested UI action
- **THEN** the GUI shows a visible failure or timeout state
- **AND** the initiating control becomes usable again when recovery is expected

### Requirement: Desktop UI test findings are recorded before product fixes
When expanded desktop UI tests reveal product behavior defects, the defect SHALL be recorded in the change findings log before any product code fix is made.

#### Scenario: Test-discovered product issue is logged
- **WHEN** a new desktop UI test exposes a product UI behavior defect that is not already explicitly in scope for fixing
- **THEN** the issue is recorded in `openspec/changes/expand-desktop-ui-test-coverage/findings.md`
- **AND** product UI code is not changed for that defect unless explicitly requested
