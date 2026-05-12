## ADDED Requirements

### Requirement: Approve tasks support explicit review mode
The system SHALL allow an `approve` task to run in explicit review mode.

When an explicit-review approve task receives and validates a `join_request`, it SHALL persist a pending approval request instead of immediately delivering a `membership_bundle`.

The pending approval request SHALL include at minimum: `approve_task_id`, `invite_id`, `request_msg_id`, `member_peer_id`, `status`, `created_at`, and any non-secret joiner display hints that were included in the signed request.

While a request is pending, the system SHALL NOT decrement `uses_left` and SHALL NOT publish a `membership_bundle` for that `request_msg_id`.

Duplicate `join_request` messages with the same `request_msg_id` SHALL update or reuse the same pending approval request instead of creating another pending request.

The system SHALL persist private, validated decision material needed to resolve and publish decisions after daemon restart while the invite is still unexpired. This material SHALL include the request reply topic, validated join request body, member public keys, and invite brokers, and SHALL NOT be exposed through desktop runtime state, LocalAPI responses, task reports, or SSE events.

#### Scenario: Join request waits for explicit approval
- **WHEN** an explicit-review `approve` task receives a valid `join_request`
- **THEN** the system records one pending approval request for that `request_msg_id`
- **AND** no membership bundle is published until an approval decision is accepted
- **AND** invite uses are not decremented while the request is pending

#### Scenario: Duplicate pending request is coalesced
- **WHEN** an explicit-review `approve` task receives the same valid `join_request` more than once
- **THEN** the system exposes one pending approval request for that `request_msg_id`
- **AND** duplicate delivery does not consume another invite use

#### Scenario: Pending request survives daemon restart
- **GIVEN** an explicit-review `approve` task has recorded a pending approval request
- **WHEN** the daemon restarts before the request is decided
- **THEN** the pending request remains visible as an approval request
- **AND** the request remains decision-addressable by `approve_task_id` and `request_msg_id`

### Requirement: Approval decisions are task-addressed and idempotent
The system SHALL support an approval decision task that targets a pending request by `approve_task_id` and `request_msg_id`.

The decision task SHALL accept exactly one decision value: `approve` or `reject`.

The decision task SHALL resolve persisted pending requests without requiring the original `approve` task runtime to still be active.

When the decision is `approve`, the system SHALL apply existing invite idempotency and uses accounting, publish the encrypted `membership_bundle`, cache the response for duplicate `request_msg_id` handling, and mark the approval request `approved`.

When the decision is `reject`, the system SHALL persist a terminal rejection, publish an encrypted rejection response with no membership bundle, and mark the approval request `rejected` without decrementing `uses_left`.

After a request reaches a terminal decision, repeating the same decision SHALL return the existing terminal result without changing invite uses. A conflicting later decision SHALL fail without changing the prior terminal decision.

#### Scenario: Approve decision delivers membership
- **GIVEN** an explicit-review approve task has a pending approval request
- **WHEN** an approval decision task accepts that request
- **THEN** the system publishes the membership bundle to the request reply topic
- **AND** decrements invite uses at most once for that `request_msg_id`
- **AND** the approval request becomes `approved`

#### Scenario: Approve decision after restart delivers membership
- **GIVEN** an explicit-review approve task recorded a pending approval request before daemon restart
- **WHEN** an approval decision task accepts that request after restart
- **THEN** the system publishes the membership bundle to the persisted request reply topic
- **AND** decrements invite uses at most once for that `request_msg_id`
- **AND** the approval request becomes `approved`

#### Scenario: Reject decision denies membership without consuming uses
- **GIVEN** an explicit-review approve task has a pending approval request
- **WHEN** an approval decision task rejects that request
- **THEN** the system publishes a terminal rejection response without a membership bundle
- **AND** invite uses are not decremented for that rejection
- **AND** the approval request becomes `rejected`

#### Scenario: Reject decision after restart denies membership
- **GIVEN** an explicit-review approve task recorded a pending approval request before daemon restart
- **WHEN** an approval decision task rejects that request after restart
- **THEN** the system publishes a terminal rejection response to the persisted request reply topic
- **AND** invite uses are not decremented for that rejection
- **AND** the approval request becomes `rejected`

#### Scenario: Conflicting terminal decision is ignored
- **GIVEN** an approval request has already been approved or rejected
- **WHEN** another decision task submits the opposite decision for the same `request_msg_id`
- **THEN** the task fails without changing the prior decision
- **AND** invite uses remain unchanged
