## ADDED Requirements

### Requirement: LocalAPI exposes typed approval request runtime state
`GET /api/v0/desktop/state` SHALL include approval requests as typed JSON objects suitable for desktop review workflows.

Each approval request object SHALL include at minimum: `approve_task_id`, `invite_id`, `request_msg_id`, `member_peer_id`, `status`, `created_at`, and `updated_at` when available.

Approval request objects MUST NOT include invite secrets, net secrets, private keys, decrypted membership bundles, or raw encrypted payloads.

Approval request objects MUST NOT include private restart decision material such as invite brokers, reply topics, validated join request bodies, or member public key payloads.

Desktop SSE SHALL publish `approval_requests.replace` when approval request state changes.

#### Scenario: Pending approval appears in desktop state
- **WHEN** an explicit-review approve task records a pending join request
- **THEN** `GET /api/v0/desktop/state` includes that request in `approval_requests`
- **AND** the request can be addressed by `approve_task_id` and `request_msg_id`

#### Scenario: Approval updates are streamed
- **WHEN** an approval request is created, approved, rejected, or expires
- **THEN** the desktop event stream emits an `approval_requests.replace` update
- **AND** the update does not expose secret material

### Requirement: LocalAPI supports approval decisions through task creation
`POST /api/v0/tasks` SHALL support task kind `approve_decision`.

The `approve_decision` task args SHALL include `approve_task_id`, `request_msg_id`, and `decision`.

The task SHALL fail without side effects when the referenced approve task or request does not exist, when the decision value is invalid, or when the request has already reached a conflicting terminal decision.

The task SHALL support pending requests that were persisted before daemon restart and SHALL NOT require the referenced `approve` task to still be active.

#### Scenario: Desktop client submits an approve decision
- **GIVEN** LocalAPI has a pending approval request
- **WHEN** a client creates an `approve_decision` task with `decision="approve"`
- **THEN** the task applies the approval decision to the referenced request
- **AND** task state reports the final decision result

#### Scenario: Desktop client submits a decision after daemon restart
- **GIVEN** LocalAPI has a persisted pending approval request from before daemon restart
- **WHEN** a client creates an `approve_decision` task for that request
- **THEN** the task applies the approval decision without an active `approve` runtime
- **AND** desktop state reports the final decision result

#### Scenario: Invalid decision is rejected
- **WHEN** a client creates an `approve_decision` task with an invalid decision value
- **THEN** the task fails without changing any approval request
- **AND** diagnostics identify the invalid decision
