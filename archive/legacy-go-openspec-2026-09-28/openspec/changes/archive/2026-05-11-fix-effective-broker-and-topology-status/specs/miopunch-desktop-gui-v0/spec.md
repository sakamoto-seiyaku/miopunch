## ADDED Requirements

### Requirement: Peer status distinguishes selected targets from active connections
The desktop GUI SHALL distinguish a peer selected as a target neighbor candidate from a peer with an active connection.

The GUI SHALL reserve `active` for peers that have an active topology edge. A selected but inactive peer SHALL be labeled as a target candidate or equivalent non-connected wording, and peer detail SHALL show recent failure evidence when available.

#### Scenario: Selected peer is not shown as connected
- **WHEN** topology contains a peer in `neighbors.selected` but not in `neighbors.active`
- **THEN** the GUI does not label that peer as active or connected
- **AND** the peer detail indicates it is only a selected target candidate

#### Scenario: Recent failure is visible for inactive selected peer
- **WHEN** topology contains a failed attempt for a selected peer and no active edge for that peer
- **THEN** the peer detail shows the failure stage, reason code, or stop condition when available
