## ADDED Requirements

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

## MODIFIED Requirements

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
