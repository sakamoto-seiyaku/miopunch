# miopunch-desktop-gui-v0 Specification

## Purpose
TBD - created by archiving change client-win-linux. Update Purpose after archive.
## Requirements
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
- **WHEN** the GUI cannot reach any default LocalAPI endpoint
- **AND** starting the same-user session daemon fails or times out
- **THEN** the GUI shows a session bootstrap failure
- **AND** the default view suggests retrying, checking the sibling `miopunch` binary, and exporting runtime diagnostics
- **AND** the default view does not require `miopunch install-system-daemon`

#### Scenario: Unknown/incompatible endpoint suggests version and bundle checks
- **WHEN** the GUI reaches an endpoint but receives an unexpected response
- **THEN** the GUI indicates the failure may be caused by version mismatch or endpoint mismatch
- **AND** the default view suggests checking that `miopunch` and `miopunch-desktop` came from the same bundle and exporting runtime diagnostics

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

### Requirement: Desktop GUI uses one authoritative runtime bootstrap and resync path
The desktop GUI SHALL bootstrap runtime state through one desktop runtime bridge
path instead of composing a live view from multiple bridge calls.

The runtime bootstrap path SHALL:
- connect to LocalAPI
- return one authoritative desktop runtime snapshot
- start the live desktop runtime event stream

The GUI SHALL use an explicit resync path for manual refresh and revision-gap
recovery.

#### Scenario: Startup uses one runtime bootstrap call
- **WHEN** the desktop GUI finishes loading and registers its runtime listeners
- **THEN** it starts runtime state through one bridge call
- **AND** it does not separately fetch `status`, `peers`, `topology`, and `tasks` to establish the same initial state

#### Scenario: Manual refresh uses one resync path
- **WHEN** the user triggers Refresh in the desktop GUI
- **THEN** the GUI requests one desktop runtime resync
- **AND** the GUI does not re-run a piecemeal per-slice fetch chain

### Requirement: GUI renders snapshot-first state and stays updated in near real-time
The GUI SHALL display the daemon state including product-facing runtime state
for `status`, `topology`, `peer_sessions`, `config`, `diagnostics`,
`shell_sessions`, and task history.

The `config` subtree SHALL include the existing local/known-peer/net runtime
summary and the Settings runtime config model for desired/effective config,
desktop preferences, validation/apply metadata, and safe diagnostics.

The GUI SHALL keep the view updated in near real-time by consuming the desktop
runtime state stream and applying ordered revisioned updates to a single
authoritative client store.

Task SSE MAY remain available for debug/detail flows, but the main desktop UI
SHALL NOT depend on task-event stitching or manual refresh to keep primary
runtime state current.

#### Scenario: UI shows initial desktop snapshot then applies typed updates
- **GIVEN** LocalAPI desktop runtime state endpoints are available
- **WHEN** the GUI opens the main screen
- **THEN** the UI renders an initial desktop runtime snapshot
- **AND** later runtime state updates apply directly to the visible UI without requiring a manual refresh

#### Scenario: Revision gap falls back to one resync
- **WHEN** the GUI detects that a desktop runtime event does not continue from the previously applied revision
- **THEN** the GUI discards incremental assumptions
- **AND** the GUI reloads one fresh desktop runtime snapshot
- **AND** the visible UI does not require the user to manually reconnect or refresh to recover

### Requirement: Desktop Settings manages runtime config through the daemon
The desktop GUI SHALL provide a Settings runtime config surface backed by
daemon-authoritative desktop runtime state and bridge methods.

The Settings runtime config surface SHALL cover current runtime fields:
- MQTT broker endpoints
- `p2p_network`
- `p2p_ip_family`
- `data_proto`
- `quic_cc`
- STUN endpoints
- portmap and assisted-address toggles
- default shell target/session
- log level

The GUI SHALL distinguish desired persisted config from effective runtime
config and SHALL show whether each saved change applies immediately, applies to
future connections, or needs active sessions to reconnect.

The GUI SHALL NOT directly read or write daemon state files or logs.

#### Scenario: Settings shows desired and effective runtime config
- **WHEN** the desktop runtime snapshot includes config state
- **THEN** Settings shows desired and effective values for the supported fields
- **AND** it indicates the current apply status

#### Scenario: User saves valid runtime config
- **WHEN** the user changes a supported config field and clicks Save
- **THEN** the GUI calls the desktop bridge config save method
- **AND** the visible state updates from the returned snapshot or a later
  `config.replace` event

#### Scenario: Validation errors are visible and recoverable
- **WHEN** the daemon rejects a Settings save request
- **THEN** the GUI shows the structured failure summary and suggestions
- **AND** the Save control becomes available again

### Requirement: Desktop Settings exports redacted diagnostics
The desktop GUI SHALL allow exporting runtime diagnostics to a user-selected
archive path through the Go bridge.

The exported diagnostics SHALL include redacted runtime state and available
desktop/daemon logs. The GUI SHALL show cancellation, success path, and failure
states.

#### Scenario: User exports diagnostics
- **WHEN** the user clicks Export diagnostics and chooses a path
- **THEN** the Go bridge writes a redacted diagnostics archive to that path
- **AND** the GUI confirms success and shows the output path

#### Scenario: Diagnostics export can be cancelled
- **WHEN** the user cancels the save dialog
- **THEN** the GUI remains usable
- **AND** no failure toast is shown

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
The desktop GUI SHALL include CI-run browser tests that verify runtime state
events and bridge failures update the UI predictably.

#### Scenario: Runtime state events update rendered desktop state
- **WHEN** the fake desktop runtime emits an initial snapshot and later typed state updates
- **THEN** the GUI updates the visible runtime state without a manual refresh

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

### Requirement: First-run desktop exposes network setup entry points
When the desktop GUI is connected to a blank uninitialized node, it SHALL expose
both new-network and existing-network setup paths.

A blank uninitialized node is one whose topology has no net ID, no governance
head, no decls head, no members, and a missing or `unknown` self role.

The GUI MAY treat that blank first-run node as an owner candidate for UI
visibility only. This SHALL NOT require daemon startup to create network,
governance, or declaration state.
This SHALL NOT imply that runtime broker state such as `brokers_effective`
already exists before the user starts `invite/create` or completes `join`.

#### Scenario: Blank node can create or join from Access
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Access shows Join network
- **AND** Access shows Create invite
- **AND** Access shows Approve request

#### Scenario: Blank node can open Admin before network creation
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Admin navigation is available
- **AND** the local self row is displayed as an owner candidate

#### Scenario: Joined member remains restricted
- **WHEN** the desktop GUI loads topology for a node whose self role is `member`
- **THEN** admin-only navigation and Access flows remain hidden

### Requirement: Desktop invite creation handles asynchronous task output
The desktop GUI SHALL render the generated invite code when an invite task produces the `invite_code` fact after the initial task creation response.

#### Scenario: Invite code arrives through a later task fetch
- **WHEN** the user triggers the invite Create action and the created task initially has no `invite_code` fact
- **AND** a later task fetch includes the `invite_code` fact
- **THEN** the GUI renders the invite code
- **AND** the Copy action becomes available
- **AND** the QR code area renders a QR representation of the invite code

#### Scenario: Invite code arrives through a runtime task event
- **WHEN** the user is viewing the invite flow for a created invite task
- **AND** a runtime task event supplies an `invite_code` fact for that task
- **THEN** the GUI renders the invite code without requiring a manual refresh

#### Scenario: Invite code arrives through a runtime task snapshot
- **WHEN** the user is viewing the invite flow for a created invite task
- **AND** a final runtime task event includes a task snapshot containing an `invite_code` fact
- **THEN** the GUI renders the invite code without requiring a manual refresh
- **AND** placeholder suggestions such as `miopunch join <invite_code>` are not treated as a usable code

#### Scenario: Successful invite completion without code is visible
- **WHEN** an invite task reaches `done` with reason `OK` but no `invite_code` fact is available
- **THEN** the GUI shows a visible diagnostic that the invite code is missing from task output
- **AND** the Copy action remains unavailable

### Requirement: Peer status distinguishes selected targets from active connections
The desktop GUI SHALL distinguish a peer selected as a target neighbor candidate from a peer with an active connection.

The GUI SHALL reserve `active` for peers that have an active topology edge. A selected but inactive peer SHALL be labeled as a target candidate or equivalent non-connected wording, and peer detail SHALL show recent failure evidence when available.

#### Scenario: Selected peer is not shown as connected
- **WHEN** topology contains a peer in `neighbors.selected` but not in `neighbors.active`
- **THEN** the GUI does not label that peer as active or connected
- **AND** the peer detail indicates it is only a selected target candidate

#### Scenario: Recent failure is visible for inactive selected peer
- **WHEN** topology contains a failed attempt for a selected peer and no active edge for that peer
- **THEN** the peer detail shows the failure stage, reason code, or stop condition when available

### Requirement: Access renders pending approval requests for owner/admin users
The desktop GUI SHALL render pending approval requests from desktop runtime state in the Access tab for owner/admin users.

Each pending request SHALL show the joiner peer identity, available non-secret display hints, request status, and decision actions.

Member users SHALL NOT see approval request decision controls.

#### Scenario: Owner sees pending request actions
- **WHEN** the desktop runtime state includes a pending approval request
- **AND** the local self role is owner or admin
- **THEN** Access shows the request with Approve and Reject actions
- **AND** the visible request does not expose secret material

#### Scenario: Member cannot decide approval requests
- **WHEN** the desktop UI is opened as a member role
- **THEN** approval request decision controls are unavailable

### Requirement: Access submits approval decisions through the bridge task path
When an owner/admin user approves or rejects a pending request, the desktop GUI SHALL create an `approve_decision` task with object args containing `approve_task_id`, `request_msg_id`, and `decision`.

The GUI SHALL show decision progress and SHALL update the visible request when runtime state reports approved, rejected, or expired status.

#### Scenario: Approve action creates decision task
- **GIVEN** Access shows a pending approval request
- **WHEN** the user clicks Approve
- **THEN** the desktop bridge creates an `approve_decision` task with `decision="approve"`
- **AND** the UI shows decision progress until runtime state updates the request

#### Scenario: Reject action creates decision task
- **GIVEN** Access shows a pending approval request
- **WHEN** the user clicks Reject
- **THEN** the desktop bridge creates an `approve_decision` task with `decision="reject"`
- **AND** the UI shows decision progress until runtime state updates the request

### Requirement: Approval decision failures are visible and recoverable
The desktop GUI SHALL make approval decision controls recoverable when the bridge call fails, times out, or the decision task fails.

After a recoverable failure, the relevant decision control SHALL become usable again if the request is still pending.

#### Scenario: Decision bridge failure is visible
- **GIVEN** Access shows a pending approval request
- **WHEN** the user submits a decision and the bridge returns an error
- **THEN** the GUI shows a visible failure message
- **AND** the request remains actionable if it is still pending

#### Scenario: Decision task failure is visible
- **GIVEN** Access shows a pending approval request
- **WHEN** the decision task reaches a failed terminal state
- **THEN** the GUI shows the task failure reason or diagnostic detail
- **AND** the visible approval request follows the latest runtime state

### Requirement: Embedded shell terminal focuses input when opened
The desktop GUI SHALL focus the embedded shell terminal input when the terminal
is opened for a shell session.

The user SHALL be able to start typing into a newly opened shell session
without an extra click inside the terminal area.

#### Scenario: Connected shell is ready for immediate input
- **WHEN** the user opens an embedded shell session in the desktop GUI
- **AND** the shell reaches its connected terminal view
- **THEN** the terminal input is focused
- **AND** keyboard input can be sent immediately without a separate focus click

### Requirement: Desktop shell abnormal close shows structured diagnostics
The desktop GUI SHALL surface the best available structured diagnostic summary
when an embedded desktop shell closes unexpectedly after attach setup has
started, instead of showing only a generic disconnect string.

The summary SHALL prefer final shell task diagnostics when available and SHALL
preserve the selected peer, target, and session so the operator can retry the
same shell path.

The summary SHALL distinguish user-requested disconnect from abnormal shell
termination.

#### Scenario: Unexpected shell close uses final task diagnostics
- **GIVEN** the desktop GUI has attached an embedded shell for a selected peer,
  target, and session
- **WHEN** the shell closes unexpectedly and the final `sh_attach` task exposes
  structured diagnostic output
- **THEN** the desktop shell view shows a concise failure summary derived from
  that diagnostic output instead of only a raw WebSocket close string
- **AND** Connect remains available for the same peer, target, and session

#### Scenario: Explicit disconnect is not shown as a failure
- **GIVEN** the desktop GUI has an active embedded shell
- **WHEN** the user explicitly disconnects that shell
- **THEN** the shell view returns to a disconnected state without an abnormal
  failure banner
- **AND** the same peer, target, and session remain selected for reconnect

### Requirement: Desktop shell supports a repeatable demo loop
The desktop GUI SHALL provide a repeatable shell demo loop using the existing
`sh_ls` and `sh_attach` task contracts.

The loop SHALL allow a user to select an operable peer, list available shell
targets or sessions, choose target and session values, connect an embedded
terminal, disconnect, and reconnect without leaving the peer shell view.

When discovery has not returned a richer value, the GUI SHALL default to target
`local` and session `main`.

#### Scenario: User lists sessions before connecting
- **GIVEN** a remote peer is operable
- **WHEN** the user opens the peer shell view and starts discovery
- **THEN** the GUI creates an `sh_ls` task for that peer
- **AND** the shell view shows available target or session choices when the task
  result provides them

#### Scenario: User connects, disconnects, and reconnects
- **GIVEN** a remote peer is operable
- **WHEN** the user connects a shell with selected target and session values
- **THEN** the GUI creates an `sh_attach` task with those values
- **AND** the embedded terminal shows connected status
- **WHEN** the user disconnects
- **THEN** the terminal transport is closed and the shell view can reconnect
  with the same selected values

### Requirement: Desktop shell failures are visible and recoverable
The desktop GUI SHALL show recoverable failure states for shell discovery,
`sh_attach` task creation, terminal bridge setup, WebSocket close, and terminal
library load failures.

After a recoverable failure, the shell view SHALL keep the selected peer,
target, and session values available for retry.

#### Scenario: Discovery failure keeps retry available
- **WHEN** the user starts shell discovery and the `sh_ls` task or bridge call
  fails
- **THEN** the GUI shows a visible failure message
- **AND** discovery can be retried for the same peer

#### Scenario: Attach failure keeps retry available
- **WHEN** the user starts shell attach and task creation or terminal bridge
  setup fails
- **THEN** the GUI shows a visible failure message
- **AND** Connect becomes available again for the same peer, target, and session

### Requirement: Desktop shell demo loop has browser coverage
The desktop GUI SHALL include browser-level tests for the committed static UI
shell demo loop.

The tests SHALL use fake bridge/runtime behavior and SHALL NOT require live
network transport.

#### Scenario: Browser test covers successful shell loop
- **WHEN** the browser test opens an operable peer shell view
- **THEN** it can run discovery, connect `sh_attach`, observe terminal status,
  disconnect, and reconnect
- **AND** the fake bridge records the expected `sh_ls` and `sh_attach` task calls

#### Scenario: Browser test covers shell failure recovery
- **WHEN** the fake bridge fails discovery or attach setup
- **THEN** the browser test observes a visible failure state
- **AND** the shell control can be retried without navigating away
