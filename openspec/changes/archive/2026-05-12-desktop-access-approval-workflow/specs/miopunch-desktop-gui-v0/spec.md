## ADDED Requirements

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
