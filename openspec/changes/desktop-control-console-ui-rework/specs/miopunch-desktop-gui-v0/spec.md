## ADDED Requirements

### Requirement: Desktop console uses Network Shell Admin Settings navigation
The desktop GUI SHALL expose `Network`, `Shell`, `Admin`, and `Settings` as the primary console navigation.

The GUI SHALL NOT expose `Access` as a primary tab. A URL or saved deep link that requests `Access` SHALL route to the supported console flow.

#### Scenario: Access deep link redirects to Network
- **WHEN** the desktop GUI opens with a deep link requesting the Access tab
- **THEN** the visible primary tab is Network
- **AND** no Access primary tab is rendered

#### Scenario: Member console shows allowed primary tabs
- **WHEN** the desktop GUI loads runtime state for a joined member node
- **THEN** Network is available
- **AND** Shell is available
- **AND** Admin is unavailable

#### Scenario: Owner console shows Admin
- **WHEN** the desktop GUI loads runtime state for an owner or admin node in a joined network
- **THEN** Admin is available
- **AND** invite creation and approval review are reachable from Admin

### Requirement: Network detail persists desktop-local peer aliases
The desktop GUI SHALL let the user save a local alias for a peer through the desktop Settings/config bridge path.

The alias SHALL be displayed ahead of the remote member name, but the peer ID and remote member name SHALL remain visible in peer detail.

#### Scenario: User saves a peer alias
- **WHEN** the user saves a local alias for a peer from Network detail
- **THEN** the GUI calls the desktop config save bridge with the alias in desktop preferences
- **AND** the visible peer title uses the alias after the returned snapshot or config event applies
- **AND** the remote member name and peer ID remain visible

#### Scenario: User clears a peer alias
- **WHEN** the user saves an empty local alias for a peer
- **THEN** the alias is cleared from desktop preferences
- **AND** the visible peer title falls back to the remote member name or peer ID

### Requirement: Live desktop console avoids preview-only runtime evidence
When the desktop GUI is connected to a live daemon, it SHALL render device names, path details, shell sessions, and connection health from the desktop runtime snapshot and events.

Prototype-only device names, endpoint facts, and metrics SHALL NOT be used in live mode. Missing daemon evidence SHALL be shown as unknown or not measured.

#### Scenario: Live peer name uses runtime identity data
- **WHEN** live topology includes a peer with no alias and a remote member name
- **THEN** the GUI displays the remote member name
- **AND** it does not use a prototype-only device name for that peer

#### Scenario: Missing path details are honest
- **WHEN** live topology and peer-session state do not include endpoint or metric data
- **THEN** Network detail displays unknown or not measured values
- **AND** it does not display preview-only endpoint, tuple, RTT, throughput, or loss values

### Requirement: Shell Resume is gated by attachable runtime sessions
The desktop GUI SHALL enable Resume only for shell sessions whose runtime state reports that the local task is attachable.

When a session is not attachable, the GUI SHALL keep it visible as historical or running context but SHALL require opening another shell instead of attempting Resume.

#### Scenario: Attachable shell session can resume
- **WHEN** the Shell workspace lists a session with `attachable=true`
- **THEN** the Resume action is enabled
- **AND** activating Resume attaches to that existing shell task instead of creating a new task

#### Scenario: Non-attachable shell session cannot resume
- **WHEN** the Shell workspace lists a session with `attachable=false`
- **THEN** the Resume action is disabled
- **AND** the GUI offers opening another shell for the same peer, target, and session values

### Requirement: Admin invite creation starts approval review automatically
When an owner/admin creates an invite from Admin, the desktop GUI SHALL start an explicit approval listener for the generated invite code without requiring manual code paste.

The refresh action SHALL remain only a manual recovery resync and SHALL NOT be required for normal approval request visibility.

#### Scenario: Invite starts listener
- **WHEN** an owner/admin creates an invite
- **AND** the invite task returns an invite code
- **THEN** the GUI creates an `approve` task with `explicit_review=true` for that code
- **AND** incoming approval request events appear in Admin without pressing Refresh

### Requirement: Shell workspace keeps terminal primary
The Shell workspace SHALL keep the terminal as the primary surface while making targets and sessions collapsible.

Target discovery SHALL populate target entry choices and SHALL NOT flood the left target list with every discovered target. Creating an additional session SHALL require a session name, while resuming an existing attachable session SHALL reuse the existing task.

#### Scenario: Target discovery stays in choices
- **WHEN** Shell target discovery returns multiple targets
- **THEN** the target input choices include those targets
- **AND** the left target list remains limited to selected or already-open targets

#### Scenario: User can maximize terminal
- **WHEN** the user activates Zen mode
- **THEN** the target and session side panels are hidden
- **AND** the terminal receives the available workspace width

## MODIFIED Requirements

### Requirement: Desktop GUI has automated browser coverage for primary navigation
The desktop GUI SHALL include CI-run browser tests that verify the committed static UI can navigate between primary tabs and their second-level desktop views without JavaScript errors.

#### Scenario: Primary tabs render their overview pages
- **WHEN** the browser test opens the desktop UI with a fake owner bridge and selects Network, Shell, Admin, and Settings
- **THEN** each tab renders its overview content
- **AND** no browser page error or unexpected console error is emitted

#### Scenario: Second-level views use the desktop interaction model
- **WHEN** the browser test opens a peer, shell workspace, admin flow, member detail, or settings section
- **THEN** the selected detail view renders within the current primary tab
- **AND** returning to Overview restores the primary tab overview

### Requirement: First-run desktop exposes network setup entry points
When the desktop GUI is connected to a blank uninitialized node, it SHALL expose the existing-network setup path under Network.

A blank uninitialized node is one whose topology has no net ID, no governance head, no decls head, no members, and a missing or `unknown` self role.

The GUI MAY treat that blank first-run node as an owner candidate for limited UI copy only. This SHALL NOT require daemon startup to create network, governance, or declaration state. This SHALL NOT imply that runtime broker state such as `brokers_effective` already exists before the user completes `join`.

#### Scenario: Blank node can join from Network
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Network shows Join network
- **AND** no Access primary tab is rendered

#### Scenario: Blank node keeps Admin hidden by default
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **THEN** Admin navigation is unavailable
- **AND** the visible primary flow remains Network setup

#### Scenario: Blank node can enable owner admin mode from Settings
- **WHEN** the desktop GUI loads topology for a blank uninitialized node
- **AND** the user opens Settings
- **AND** the user enables Owner/Admin mode
- **THEN** Admin navigation is available
- **AND** the Admin flow can create an invite for initializing the network
- **AND** the Network Join flow remains unchanged

#### Scenario: Joined member remains restricted
- **WHEN** the desktop GUI loads topology for a node whose self role is `member`
- **THEN** admin-only navigation and Admin flows remain hidden

## REMOVED Requirements

### Requirement: Access renders pending approval requests for owner/admin users
**Reason**: Approval review moved from the removed Access primary tab into Admin.

**Migration**: Use Admin approval review for owner/admin users.

### Requirement: Access submits approval decisions through the bridge task path
**Reason**: Approval decision submission moved from the removed Access primary tab into Admin while preserving the same bridge task contract.

**Migration**: Submit approval decisions from Admin using the existing `approve_decision` task path.
