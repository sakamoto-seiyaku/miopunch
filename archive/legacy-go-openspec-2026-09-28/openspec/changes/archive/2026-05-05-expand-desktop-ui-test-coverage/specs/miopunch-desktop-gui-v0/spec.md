## ADDED Requirements

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
