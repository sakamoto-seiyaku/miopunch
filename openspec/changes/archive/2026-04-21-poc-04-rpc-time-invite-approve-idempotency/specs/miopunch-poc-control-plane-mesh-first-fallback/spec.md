## MODIFIED Requirements

### Requirement: Dual-path delivery does not cause duplicate side effects
If a receiver observes the same message via multiple delivery paths (e.g., both mesh and MQTT), it SHALL apply at most one set of side effects for that message ID.

If the message is an RPC request (when `signed.kind` ends with `_request`) and the receiver has already produced a final response for that `request_msg_id`, the receiver SHALL re-send the cached final response on duplicate deliveries.

#### Scenario: Receiver processes duplicate deliveries at most once
- **WHEN** a receiver receives the same `msg_id` via mesh and MQTT within the dedup window
- **THEN** it processes the message at most once

#### Scenario: Receiver re-sends cached final response for duplicate RPC request
- **GIVEN** a receiver already produced a final response for `request_msg_id=X`
- **WHEN** it receives a duplicate RPC request again with `route.msg_id=X`
- **THEN** it re-sends the cached final response
