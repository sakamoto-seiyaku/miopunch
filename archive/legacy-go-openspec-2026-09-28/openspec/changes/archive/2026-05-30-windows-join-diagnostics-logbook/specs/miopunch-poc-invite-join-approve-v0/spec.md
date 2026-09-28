# miopunch-poc-invite-join-approve-v0 Specification Delta

## ADDED Requirements

### Requirement: enroll failures expose broker and topic facts

`invite` / `approve` / `join` SHALL expose non-secret broker and topic facts that are sufficient to diagnose signaling-stage failures from logs, LocalAPI responses, and desktop diagnostics.

At minimum, the action result or failure facts SHALL include the relevant subset of:

- `network_id`
- `invite_id`
- `broker_endpoint`
- `join_topic`
- `reply_topic`
- local or approved `peer_id`

These facts SHALL NOT include raw private keys, mailbox secrets, invite secrets, tokens, or raw invite codes beyond the existing successful invite result surface.

#### Scenario: join signaling open failure reports broker context

- **WHEN** `join` fails before opening the MQTT signaling session
- **THEN** the failure includes `network_id`, `invite_id`, `join_topic`, `broker_endpoint`, and the local joiner `peer_id`

#### Scenario: join publish failure reports reply topic context

- **WHEN** `join` opens the signaling session but fails to publish the join request
- **THEN** the failure includes `network_id`, `invite_id`, `join_topic`, `reply_topic`, `broker_endpoint`, and the local joiner `peer_id`

#### Scenario: join timeout reports broker and topic context

- **WHEN** `join` publishes the join request but times out waiting for the enroll response
- **THEN** the failure includes `network_id`, `invite_id`, `join_topic`, `reply_topic`, `broker_endpoint`, and the local joiner `peer_id`

#### Scenario: approve signaling open failure reports invite context

- **WHEN** `approve` fails before opening the join-topic signaling session
- **THEN** the failure includes `network_id`, `invite_id`, `join_topic`, and `broker_endpoint`
