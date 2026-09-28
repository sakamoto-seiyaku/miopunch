## MODIFIED Requirements

### Requirement: First-run desktop exposes network setup entry points
When the desktop GUI is connected to a blank uninitialized node, it SHALL expose
the existing-network setup path under Network and a daemon-backed owner/admin
initialization path under Settings.

A blank uninitialized node is determined from daemon-provided governance
capabilities, not frontend-only state.

#### Scenario: Blank node can join from Network
- **WHEN** the desktop GUI loads governance state `no_network`
- **THEN** Network shows Join network
- **AND** Admin navigation is unavailable until owner/admin initialization
  succeeds

#### Scenario: Blank node initializes owner admin mode from Settings
- **WHEN** the desktop GUI loads governance state `no_network`
- **AND** the user initializes Owner/Admin mode from Settings
- **THEN** the GUI creates an `init_network` task with `mode=bootstrap`
- **AND** after the daemon reports success, Admin navigation is available
- **AND** Admin invite creation uses the persisted daemon network state

#### Scenario: Existing non-admin state cannot self-promote
- **WHEN** the desktop GUI loads a governance state other than `no_network` and
  without invite/approve capability
- **THEN** the GUI does not offer Owner/Admin initialization for that existing
  network
- **AND** Admin invite and approve flows remain unavailable

#### Scenario: Non-admin user can create a new local network after confirmation
- **WHEN** the desktop GUI loads member, foreign, or stale local governance
  state
- **AND** the user confirms creating a new network
- **THEN** the GUI creates an `init_network` task with `mode=create_new` and the
  required confirmation
- **AND** after success, the GUI shows the new admin-capable network

### Requirement: Admin invite creation starts approval review automatically
When an owner/admin creates an invite from Admin, the desktop GUI SHALL start an
explicit approval listener for the generated invite code without requiring
manual code paste.

The GUI SHALL only expose this flow when daemon-provided governance
capabilities report that invite and approval actions are available.

#### Scenario: Invite starts listener
- **WHEN** an owner/admin creates an invite
- **AND** the invite task returns an invite code
- **THEN** the GUI creates an `approve` task with `explicit_review=true` for
  that code
- **AND** incoming approval request events appear in Admin without pressing
  Refresh

#### Scenario: Non-admin cannot open invite flow
- **WHEN** desktop governance capabilities report `can_invite=false`
- **THEN** the GUI does not expose Create invite
- **AND** direct Admin invite deep links are redirected to an allowed setup view
