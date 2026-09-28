## ADDED Requirements

### Requirement: approve_member carries reachability hints for peer selection
An `approve_member` declaration SHALL be able to carry `v4_hint` and `v6_hint` values for the approved member.

Reachability hints SHALL be used only for ordering bootstrap and neighbor candidates. Hints MUST NOT contain endpoint addresses, ports, or private network details.

The v4 hint order SHALL be:
`direct > easy > hard1 > hard2 > unknown > none`.

The v6 hint order SHALL be:
`direct > easy > hard1 > unknown > none`.

#### Scenario: Declaration exposes sortable hints without endpoints
- **WHEN** a node reads an `approve_member` declaration
- **THEN** it can obtain `v4_hint` and `v6_hint` values for candidate ordering
- **AND** those hint values do not expose IP addresses or ports

### Requirement: presence state includes governance and decls head summaries
Presence evidence used for MNT-03 SHALL include state-head summaries for governance and decls.

If a receiver observes divergent state-head summaries from a peer, it SHALL be able to trigger best-effort state synchronization or report the divergence as recovery evidence.

#### Scenario: Presence detects state divergence
- **WHEN** a node receives presence with different governance or decls head summaries
- **THEN** it records the divergence
- **AND** topology or recovery diagnostics can report the observed divergence
