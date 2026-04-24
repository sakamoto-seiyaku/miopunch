## MODIFIED Requirements

### Requirement: Exchange Uses The Same Program-Defined Information
MQTT signaling SHALL exchange the same program-defined information that the system already uses for traversal decisions:
`direct_addrs`, `mapped_addrs`, `assisted_addrs`,
`tcp_direct_addrs`, `tcp_mapped_addrs`, `tcp_stun_cn`, `tcp_stun_global`,
and selected transport options.

The decision logic for punching behavior SHALL remain consistent with the existing implementation.

#### Scenario: Exchange results in a usable NatHoleResp snapshot
- **WHEN** both peers complete gather and exchange via MQTT
- **THEN** each peer obtains a `NatHoleResp`-equivalent snapshot for attempt
- **AND** the snapshot includes the exchanged TCP fields when present
- **AND** the attempt step uses this snapshot to establish a usable UDP path

