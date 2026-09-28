## MODIFIED Requirements

### Requirement: Forwarding uses a dedup window to drop duplicate msg_id
The receiving/forwarding node SHALL maintain a deduplication window for message IDs:
- `seen` cache capacity: 8192 entries
- `seen` cache TTL: 10 minutes

If a node receives a message whose `msg_id` is already present in the dedup window, it SHALL apply the following rules:
- If `route.dst_peer_id != self_peer_id`, it SHALL drop the message.
- If `route.dst_peer_id == self_peer_id` and the message is not an RPC request, it SHALL drop the message.
- If `route.dst_peer_id == self_peer_id` and the message is an RPC request (when `signed.kind` ends with `_request`), it SHALL NOT drop the message solely due to the dedup window, and it SHALL make the message available for idempotent RPC handling.

#### Scenario: Duplicate msg_id is dropped for forwarded messages
- **GIVEN** a message whose `route.dst_peer_id` is not equal to `self_peer_id`
- **WHEN** a node receives the same `msg_id` twice within the dedup TTL
- **THEN** it drops the second message

#### Scenario: Duplicate RPC request for self is not dropped solely due to dedup
- **GIVEN** a message addressed to `self_peer_id` whose `signed.kind` ends with `_request`
- **WHEN** a node receives the same `msg_id` twice within the dedup TTL
- **THEN** the second delivery is made available to the RPC handling layer for idempotent response
