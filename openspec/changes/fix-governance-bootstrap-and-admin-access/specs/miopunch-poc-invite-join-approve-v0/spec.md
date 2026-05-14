## ADDED Requirements

### Requirement: Invite and approve require local admin capability
An `invite`, `approve`, or `approve_decision` task SHALL verify that the current
local identity is an owner or admin in the local governance head before it emits
an invite code, listens for join requests, records approval requests, or
publishes membership material.

The task SHALL fail locally with `FORBIDDEN` when the current identity lacks
admin capability. It SHALL include diagnostic facts identifying the self peer ID
and local governance state.

#### Scenario: Non-admin invite fails before emitting a code
- **GIVEN** a local network exists and the current identity is not owner/admin
- **WHEN** the user starts an invite task
- **THEN** the task fails with `FORBIDDEN`
- **AND** no `invite_code` fact is emitted

#### Scenario: Non-admin approve fails before publishing membership
- **GIVEN** a local network exists and the current identity is not owner/admin
- **WHEN** the user starts approve or approval-decision handling
- **THEN** the task fails with `FORBIDDEN`
- **AND** no approval declaration is added for the joiner

### Requirement: Auto invite mode is rejected until implemented
The system SHALL reject `invite --mode auto` until a complete auto-approval
design is implemented.

The task SHALL fail with `NOT_IMPLEMENTED`, SHALL NOT emit an invite code, and
SHALL suggest using approve mode.

#### Scenario: Auto invite mode is not accepted
- **WHEN** the user starts `invite` with `mode=auto`
- **THEN** the task fails with `NOT_IMPLEMENTED`
- **AND** no `invite_code` fact is emitted
- **AND** diagnostics suggest `--mode approve`
