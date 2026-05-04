## ADDED Requirements

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
