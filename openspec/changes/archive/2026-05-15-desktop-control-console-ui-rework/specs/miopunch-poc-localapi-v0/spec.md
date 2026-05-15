## ADDED Requirements

### Requirement: Desktop config supports local peer aliases
`PATCH /api/v0/desktop/config` SHALL accept desktop-local peer aliases under desktop preferences.

`GET /api/v0/desktop/state` and desktop config events SHALL return the persisted alias map in the config preferences subtree. Aliases SHALL be keyed by peer ID and SHALL NOT be written to governance declarations, member declarations, invite material, or known-peer network config.

#### Scenario: Desktop client saves a peer alias
- **WHEN** a desktop client saves `preferences.peer_aliases` for a peer
- **THEN** LocalAPI persists the alias in desktop settings
- **AND** the returned desktop runtime snapshot includes the alias
- **AND** the alias is not written to governance or declaration state

#### Scenario: Desktop client clears a peer alias
- **WHEN** a desktop client saves an empty alias for a peer
- **THEN** LocalAPI persists the cleared alias state
- **AND** later desktop snapshots do not return a non-empty alias for that peer

### Requirement: Desktop topology exposes member display hints
Desktop topology member objects SHALL include optional remote display hints derived from approved member declarations when available.

At minimum, topology members SHALL expose `member_name` and `platform` when those fields exist in the approved member declaration. These fields SHALL be non-secret display metadata and SHALL NOT replace `peer_id`.

#### Scenario: Approved member name appears in topology
- **WHEN** an approved member declaration contains `member_name` and `platform`
- **THEN** `GET /api/v0/desktop/state` includes those values on the corresponding topology member
- **AND** the member still includes its peer ID

#### Scenario: Missing member display hints are omitted
- **WHEN** an approved member declaration does not contain a member name or platform
- **THEN** desktop topology omits those optional fields
- **AND** clients can still identify the member by peer ID

### Requirement: Desktop shell sessions report local attachability
Desktop shell-session summaries SHALL report whether a local WebSocket attach can currently resume the represented `sh_attach` task.

`attachable=true` SHALL mean the task can accept or resume the foreground LocalAPI WebSocket attach. `attachable=false` SHALL mean clients must not attempt Resume for that task and should create another shell task if the user wants a new foreground terminal.

Desktop SSE SHALL publish `shell_sessions.replace` when attachability changes.

#### Scenario: Waiting shell task is attachable
- **WHEN** a `sh_attach` task is waiting for a local WebSocket attach
- **THEN** desktop runtime state includes a shell-session summary for that task
- **AND** the summary has `attachable=true`

#### Scenario: Completed shell task is not attachable
- **WHEN** a `sh_attach` task completes or its attach window is gone
- **THEN** desktop runtime state does not report it as attachable
- **AND** a desktop shell-session update is streamed when visible state changes

### Requirement: Desktop runtime exposes optional connection path details
Desktop topology and peer-session runtime state SHALL expose connection path details only when the daemon has reliable evidence for the value.

Supported optional fields include direct IPv4/IPv6 hints, local endpoint, remote endpoint, public tuple, punch status, and port. The daemon SHALL omit unknown fields instead of fabricating preview values.

#### Scenario: Reliable path details are returned
- **WHEN** the daemon has reliable endpoint or punch-status evidence for an active peer path
- **THEN** desktop runtime state includes the corresponding optional path detail fields
- **AND** the fields are available through the desktop snapshot and relevant desktop state updates

#### Scenario: Unknown path details are omitted
- **WHEN** the daemon has no reliable endpoint, tuple, punch, or metric evidence for a peer
- **THEN** desktop runtime state omits those optional fields
- **AND** it does not synthesize RTT, throughput, loss, endpoint, or tuple values
