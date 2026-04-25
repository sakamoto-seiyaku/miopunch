## MODIFIED Requirements

### Requirement: Exchange Uses The Same Program-Defined Information
MQTT signaling SHALL exchange the same program-defined information that the system already uses for traversal decisions:
`direct_addrs`, `mapped_addrs`, `assisted_addrs`,
`tcp_direct_addrs`, `tcp_mapped_addrs`, `tcp_stun_cn`, `tcp_stun_global`,
`capabilities`, `p2p_network`, and selected transport options.

The decision logic for punching behavior SHALL remain consistent with the existing implementation.

When `p2p_network=tcp_only`, exchange behavior SHALL remain consistent with the current main specification and fail fast if the peer does not advertise the required TCP Door-2 capability.

#### Scenario: Exchange results in a usable NatHoleResp snapshot
- **WHEN** both peers complete gather and exchange via MQTT
- **THEN** each peer obtains a `NatHoleResp`-equivalent snapshot for attempt
- **AND** the snapshot includes the exchanged TCP fields when present
- **AND** the attempt step uses this snapshot to establish a usable path (UDP or TCP) consistent with `p2p_network`
