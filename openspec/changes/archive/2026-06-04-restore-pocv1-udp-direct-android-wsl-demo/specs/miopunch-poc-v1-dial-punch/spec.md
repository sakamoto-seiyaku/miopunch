## MODIFIED Requirements

### Requirement: Current v1 path establishment uses UDP punching only
The system SHALL establish current POC v1 peer-to-peer paths using UDP carrier semantics only.

For IPv4 host-to-host candidate pairs, the system SHALL attempt UDP direct reachability before UDP punching.

If UDP direct reachability succeeds, the system SHALL select that UDP path and SHALL NOT run UDP punching for that pair.

If UDP direct reachability fails, the system SHALL fall back to UDP punching within the existing bounded attempt budget.

The system SHALL NOT branch to TCP, relay, QUIC, or other carrier types in this capability.

#### Scenario: Same-LAN host candidates use UDP direct first
- **WHEN** a current v1 peer tries to establish a path to an IPv4 host candidate that is directly reachable over UDP
- **THEN** it selects the UDP direct path
- **AND** it records the selected path as `direct_ipv4`
- **AND** it does not require UDP punching to converge for that pair

#### Scenario: Direct timeout falls back to UDP punching
- **WHEN** a current v1 peer tries UDP direct reachability for a host candidate pair
- **AND** the direct attempt times out
- **THEN** it attempts UDP punching for the same candidate pair
- **AND** it preserves failure evidence for the direct attempt

#### Scenario: Path establishment remains UDP-only
- **WHEN** a current v1 peer tries to establish a direct path
- **THEN** it uses only UDP direct reachability and UDP punching
- **AND** it does not use TCP, relay, QUIC, or another carrier inside this change

### Requirement: Attempt scheduling is bounded by the fixed 5B matrix
The system SHALL schedule at most 4 concurrent candidate-pair attempts and SHALL enforce a fixed 10 second total budget.

The system SHALL stop at the first successful selected path.

Each candidate-pair attempt SHALL record whether it selected `direct_ipv4`, selected `punching_ipv4`, timed out, was canceled, or failed.

#### Scenario: Attempt budget is bounded and explainable
- **WHEN** a current v1 punch run starts
- **THEN** the runtime attempts candidate pairs within the fixed 5B bounds
- **AND** it records evidence for attempted pairs, timeouts, selected path, and the selected result

#### Scenario: Attempt evidence records failure modes and convergence
- **WHEN** multiple current v1 candidate-pair attempts run in parallel
- **THEN** each failed attempt records `timeout`, `canceled`, or `failed`
- **AND** once one attempt selects the path, the remaining attempts converge and stop

### Requirement: PathResult is the only output of this capability
The system SHALL output `PathResult` from current v1 dial/punch.

`PathResult` SHALL contain only:

- the selected UDP path
- resource ownership needed by the next layer
- the trusted remote identity handoff (`peer_id` and `MemberCredential`)
- punch evidence, including the selected UDP path kind

`PathResult` SHALL be a closed handoff for `miopunch-poc-v1-secure-session`; that next layer SHALL NOT need to reopen roster lookup or dial target selection to recover remote identity.
The system SHALL NOT embed KCP/TLS/yamux or session-recipe selection into `PathResult`.

#### Scenario: Session upgrade remains outside dial/punch
- **WHEN** current v1 dial/punch succeeds
- **THEN** the result is a `PathResult`
- **AND** the session layer still chooses how to upgrade that path without re-reading roster authority
- **AND** the remote identity in that result has already passed sender, roster, and authority validation

#### Scenario: PathResult identifies the selected UDP path kind
- **WHEN** current v1 dial/punch succeeds through UDP direct reachability
- **THEN** the resulting punch evidence identifies `direct_ipv4` as the selected path
- **AND** when it succeeds through UDP punching, the resulting punch evidence identifies `punching_ipv4` as the selected path
