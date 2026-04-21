# miopunch-poc-control-plane-bounded-flooding Specification

## Purpose
`miopunch-poc-control-plane-bounded-flooding` defines the POC v0 bounded flooding (H=3) forwarding behavior, deduplication window, bounded forwarding queue, and the minimum drop facts required for diagnostics.

## Requirements

### Requirement: hop_limit is bounded to H=3
For POC v0, the system SHALL use a fixed hop limit bound `H=3`.
`route.hop_limit` SHALL be within `0..3`.
If a node receives a message with `hop_limit > 3`, it SHALL drop the message.

#### Scenario: hop_limit greater than H is dropped
- **WHEN** a node receives a message with `route.hop_limit=4`
- **THEN** it drops the message

### Requirement: Forwarding only decrements hop_limit and never changes dst_peer_id
When forwarding a message (i.e., when `route.dst_peer_id != self_peer_id`), a forwarding node SHALL:
- forward only if `route.hop_limit > 0`
- decrement `route.hop_limit` by exactly 1
- preserve `route.dst_peer_id` unchanged

The forwarding node MUST NOT modify any signed fields of the message.

#### Scenario: Forwarder decrements hop_limit and preserves dst_peer_id
- **WHEN** a forwarder forwards a message with `hop_limit=3` addressed to another peer
- **THEN** it forwards a message with `hop_limit=2`
- **AND** the forwarded message has the same `dst_peer_id` value as the original

### Requirement: Forwarder does not forward back to the source neighbor
When forwarding, the forwarder SHALL NOT send the forwarded message back to the source neighbor from which it received the message.

#### Scenario: Forwarder excludes the source neighbor from fan-out
- **WHEN** a forwarder receives a message from neighbor `Nsrc`
- **THEN** it does not forward that message to neighbor `Nsrc`

### Requirement: Forwarding uses a dedup window to drop duplicate msg_id
The receiving/forwarding node SHALL maintain a deduplication window for message IDs:
- `seen` cache capacity: 8192 entries
- `seen` cache TTL: 10 minutes

If a node receives a message whose `msg_id` is already present in the dedup window, it SHALL drop the message.

#### Scenario: Duplicate msg_id is dropped within the dedup window
- **WHEN** a node receives the same `msg_id` twice within the dedup TTL
- **THEN** it drops the second message

### Requirement: Forwarding queue is bounded and drops when full
The node SHALL bound forwarding work with a fixed forwarding queue limit:
- `forward_queue_max = 1024` messages

If the forwarding queue is full, the node SHALL drop newly received messages that require forwarding.

#### Scenario: Forwarding queue overflow drops new forwarding work
- **WHEN** the forwarding queue reaches `forward_queue_max`
- **THEN** additional inbound messages that would be forwarded are dropped

### Requirement: Node reports minimum drop facts for diagnostics
If any forwarding drops occur during an observed window (e.g., a smoke run), the node SHALL be able to report minimum diagnostic facts including:
- total forwarding drops (`mesh_forward_drops`)

#### Scenario: Drop facts include mesh_forward_drops
- **WHEN** forwarding drops occur during a run
- **THEN** the node can report a `mesh_forward_drops` fact with a non-zero value
