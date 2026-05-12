## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: GUI renders snapshot-first state and stays updated in near real-time
The GUI SHALL display the daemon state including product-facing runtime state
for `status`, `topology`, `peer_sessions`, `config`, `diagnostics`,
`shell_sessions`, and task history.

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
