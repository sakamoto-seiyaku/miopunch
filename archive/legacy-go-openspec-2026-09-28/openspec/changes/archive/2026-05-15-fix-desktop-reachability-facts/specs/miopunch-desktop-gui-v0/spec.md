## ADDED Requirements

### Requirement: Peer Details Distinguish Hints From Observed Facts
Desktop peer details SHALL label configured reachability hints separately from observed session path facts.

#### Scenario: Metadata labels are not mistaken for IP addresses
- **WHEN** the peer details view renders `v4_hint` and `v6_hint`
- **THEN** their labels identify them as reachability hints
- **AND** observed endpoint facts remain in the reachability facts section

### Requirement: Peer Details Render Available Session Path Facts
Desktop peer details SHALL render selected session path facts from `peer_sessions` or active topology neighbors when present.

#### Scenario: Endpoint facts appear for active peer
- **WHEN** LocalAPI state includes `local_endpoint`, `remote_endpoint`, `punch_status`, or `port` for the selected peer
- **THEN** the peer details reachability section displays those values instead of `unknown`
