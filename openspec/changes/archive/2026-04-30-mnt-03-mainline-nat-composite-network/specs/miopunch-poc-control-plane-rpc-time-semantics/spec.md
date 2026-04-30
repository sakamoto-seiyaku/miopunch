## ADDED Requirements

### Requirement: bootstrap_more RPC is bounded
The `bootstrap_more` RPC SHALL use bounded request/response time semantics.

A joiner SHALL wait at most 5 seconds for a `bootstrap_more_response` before treating the attempt as timed out. A joiner SHALL perform at most two `bootstrap_more` rounds for one join/rejoin sequence.

The final task report SHALL include each request ID, target admin or approver, timeout or response result, candidate count, and final stop condition.

#### Scenario: bootstrap_more stops after bounded rounds
- **WHEN** all bootstrap candidates fail and no further candidates are returned
- **THEN** the joiner stops after at most two `bootstrap_more` rounds
- **AND** the report includes the timeout or exhausted-candidates reason

### Requirement: Recovery RPCs are bounded and explainable
Recovery flows used by MNT-03, including state pull, rejoin bootstrap, and neighbor replacement requests, SHALL use bounded timeouts and retry counts.

Each recovery failure SHALL report stage, reason code, contacted peer IDs, retry budget, and stop condition.

#### Scenario: Recovery failure reports its stop condition
- **WHEN** a node cannot recover after broker outage or offline/rejoin
- **THEN** the recovery report includes contacted peers, retry budget, stage, reason, and stop condition
