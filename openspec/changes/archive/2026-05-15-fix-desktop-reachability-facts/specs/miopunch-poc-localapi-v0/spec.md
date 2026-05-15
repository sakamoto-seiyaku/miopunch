## ADDED Requirements

### Requirement: Desktop Runtime State Exposes Selected Session Path Facts
`GET /api/v0/desktop/state` SHALL include safe selected session path facts in `peer_sessions` when a live or recent peer session has that evidence.

Supported optional facts include `local_endpoint`, `remote_endpoint`, `punch_status`, and `port`. The daemon MUST omit unknown fields instead of fabricating values from reachability hints or logs.

#### Scenario: Active peer session includes endpoint facts
- **WHEN** a peer session is active and the daemon knows its selected local and remote endpoints
- **THEN** `GET /api/v0/desktop/state` includes those endpoint facts on the matching `peer_sessions` entry
- **AND** the entry still includes `remote_peer_id`, `data_proto`, `path_family`, `healthy`, and `last_activity_unix_ms`

#### Scenario: Unknown path facts remain omitted
- **WHEN** a peer session has no validated endpoint or punch-status evidence
- **THEN** `GET /api/v0/desktop/state` omits the corresponding optional fields

### Requirement: Topology Active Neighbors Mirror Session Path Facts
`GET /api/v0/topology` SHALL expose the same safe selected session path facts for active neighbors that are available in desktop peer sessions.

#### Scenario: Active neighbor includes matching session facts
- **WHEN** an active peer session has selected endpoint facts
- **THEN** the matching `topology.neighbors.active` entry includes those facts
- **AND** the facts match the `peer_sessions` entry for the same peer, protocol, and path family

### Requirement: Topology Attempts Include Selected View Evidence
Topology attempt evidence SHALL include selected STUN/TCP view and reason fields when the decision path produced them.

#### Scenario: TCP punching records selected view
- **WHEN** a TCP punching attempt succeeds and decision output includes `tcp_selected_view` and `tcp_selected_reason`
- **THEN** topology attempt evidence includes the selected view and reason for that attempt
