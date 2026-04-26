## MODIFIED Requirements

### Requirement: Exchange Uses The Same Program-Defined Information
MQTT signaling SHALL exchange the same program-defined information that the system already uses for traversal decisions:
`direct_addrs`, `mapped_addrs`, `assisted_addrs`,
`tcp_direct_addrs`, `tcp_mapped_addrs`, `tcp_stun_cn`, `tcp_stun_global`,
`capabilities`, `p2p_network`,
and selected transport options.

When `p2p_network=tcp_only`, the system SHALL fail fast during exchange if the peer capability set does not include the required TCP Door-2 capability (e.g., `tcp_p2p_v0`).

The decision logic for punching behavior SHALL remain consistent with the existing implementation (same gather snapshot, same neutral punching decision boundary, no trickle updates).

#### Scenario: Exchange results in a usable NatHoleResp snapshot
- **WHEN** both peers complete gather and exchange via MQTT
- **THEN** each peer obtains a `NatHoleResp`-equivalent snapshot for attempt
- **AND** the snapshot includes the exchanged TCP fields when present
- **AND** the attempt step uses this snapshot to establish a usable path (UDP or TCP) consistent with `p2p_network`
