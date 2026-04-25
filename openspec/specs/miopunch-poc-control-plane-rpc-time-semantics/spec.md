# miopunch-poc-control-plane-rpc-time-semantics Specification

## Purpose
`miopunch-poc-control-plane-rpc-time-semantics` defines the POC v0 time semantics and retry invariants for control-plane RPC request/response messages.

## Requirements

### Requirement: RPC requests include expires_at_unix_ms and are strictly expired
For POC v0, the system SHALL treat a control-plane message as an RPC request when `signed.kind` ends with `_request`.

An RPC request SHALL include `route.expires_at_unix_ms` (unix milliseconds).
A receiver SHALL drop an RPC request when `now_unix_ms > route.expires_at_unix_ms`.

#### Scenario: Receiver drops an expired RPC request
- **GIVEN** a receiver observes `now_unix_ms` greater than the request's `expires_at_unix_ms`
- **WHEN** it receives that RPC request
- **THEN** it drops the request

### Requirement: Receiver performs clock-skew sanity drop for large created_at divergence
The receiver SHALL treat `route.created_at_unix_ms` as a sender-provided wall-clock timestamp (unix milliseconds).
If `abs(now_unix_ms - route.created_at_unix_ms) > 10 minutes`, the receiver SHALL drop the message.

When dropping due to this condition, the receiver SHALL surface a diagnostic to upper layers indicating clock skew (e.g., "check system time").

#### Scenario: Receiver drops a message with excessive clock skew and surfaces a diagnostic
- **GIVEN** `abs(now_unix_ms - created_at_unix_ms) > 10 minutes`
- **WHEN** the receiver receives the message
- **THEN** it drops the message
- **AND** it surfaces a clock-skew diagnostic to upper layers

### Requirement: RPC retries reuse request_msg_id and do not change transcript fields except time bounds
When retrying an RPC request, the sender SHALL reuse the same `route.msg_id` value as the original request (`request_msg_id`).

Across retries of the same `request_msg_id`, the sender SHALL be allowed to change `route.created_at_unix_ms` and `route.expires_at_unix_ms`.
Across retries of the same `request_msg_id`, the sender SHALL NOT change any other signed transcript fields (`dst_peer_id`, `sender_peer_id`, `kind`, `in_reply_to`, `body`).

#### Scenario: Sender retries a request by reusing the same request_msg_id
- **WHEN** a sender retries an RPC request due to not observing a response
- **THEN** it reuses the same `route.msg_id` across retries

### Requirement: RPC responses include in_reply_to referencing request_msg_id
An RPC response SHALL set `signed.in_reply_to` to the corresponding `request_msg_id` (`route.msg_id` of the request).

#### Scenario: Response includes in_reply_to
- **WHEN** a receiver produces an RPC response for a request
- **THEN** the response includes `signed.in_reply_to` equal to the request's `route.msg_id`
