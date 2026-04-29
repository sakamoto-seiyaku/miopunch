## ADDED Requirements

### Requirement: Nodes maintain active neighbors after join
After joining a net, a node SHALL maintain active neighbors according to `k=max(2,ceil(ln(n)))`, where `n` is the known legal peer count.

Neighbor selection SHALL use reachability bucket ordering, online state, and random or rotating selection within a bucket. The system SHALL avoid selecting only the best-connected admin nodes when enough alternatives exist.

#### Scenario: Joined node chooses multiple active neighbors
- **WHEN** a node knows 12 legal peers
- **THEN** it targets approximately three active neighbors
- **AND** the selected neighbor set is not limited to the primary admin when other eligible peers exist

### Requirement: Neighbor health drives reconnect or replacement
A node SHALL monitor active neighbor health through data-plane activity, keepalive, or equivalent product-level evidence.

If an active neighbor becomes unhealthy, the node SHALL attempt bounded reconnect. If reconnect fails within budget, the node SHALL select a replacement candidate using the same reachability bucket policy.

#### Scenario: Unhealthy neighbor is replaced
- **WHEN** an active neighbor is offline long enough to fail health checks
- **THEN** the node attempts bounded reconnect
- **AND** after reconnect failure it selects a replacement candidate
- **AND** topology diagnostics report the reconnect and replacement evidence

### Requirement: Active neighbor edges carry payload evidence
An active neighbor edge SHALL be considered established only when the system has transport evidence for the edge.

For MNT-03, transport evidence SHALL include selected attempt path, data protocol, peer IDs, and a successful payload exchange or an explainable failure with stop condition.

#### Scenario: Active edge includes transport evidence
- **WHEN** a node reports a peer as an active neighbor
- **THEN** topology diagnostics include attempt path and data protocol evidence for that edge
- **AND** a gate can validate payload success or explainable failure for that edge
