## ADDED Requirements

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

#### Scenario: Successful invite completion without code is visible
- **WHEN** an invite task reaches `done` with reason `OK` but no `invite_code` fact is available
- **THEN** the GUI shows a visible diagnostic that the invite code is missing from task output
- **AND** the Copy action remains unavailable
