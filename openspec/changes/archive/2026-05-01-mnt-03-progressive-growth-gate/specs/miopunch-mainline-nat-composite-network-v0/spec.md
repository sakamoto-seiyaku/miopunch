## MODIFIED Requirements

### Requirement: MNT-03 implementation validates incremental real-node milestones
MNT-03 implementation SHALL proceed through incremental real-node checkpoints before the complete 12-node fulltest is considered valid:
`2-node substrate -> 3-node bootstrap -> 4-node reachability/portmap -> 6-node bootstrap_more/hard carry -> 8-node hard symmetry -> 12-node full topology`.

The default public MNT-03 gates SHALL validate those checkpoints as one continuously growing network. A later checkpoint MUST NOT be treated as complete until the same run has produced passing machine-readable evidence for the earlier checkpoints.

Fresh-start execution of individual 2, 3, 4, 6, or 12-node stages MAY remain available as manual/debug entry points, but those stages MUST NOT be the default scenario-three public gate path.

#### Scenario: Milestone blocks later expansion until verified
- **WHEN** the 4-node reachability checkpoint has not produced a passing summary in the current progressive run
- **THEN** the MNT-03 public gate does not treat the 6-node bootstrap_more checkpoint as complete
- **AND** artifacts identify the failing checkpoint and its evidence

### Requirement: MNT-03 provides layered gates
The lab host SHALL expose `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest`.

`mnt03-smoke` SHALL run one progressive network growth case from blank startup through the 2-node substrate and 3-node bootstrap checkpoints, including blank startup, real join, topology snapshots, and at least one successful payload edge.

`mnt03-selftest` SHALL run one progressive network growth case from blank startup through the 4-node reachability/portmap and 6-node bootstrap_more/hard carry checkpoints, including presence, reachability bucket, logN active neighbor, hard-node carry, admin hub avoidance, and active-edge evidence.

`mnt03-fulltest` SHALL run one progressive network growth case from blank startup through the complete 12-node topology and perturbation/recovery validation, including packet capture, conntrack, NAT rules, tc qdisc statistics, broker logs, daemon journals, task reports, topology snapshots, checkpoint proofs, and cleanup logs.

The public gates MUST NOT invoke separate fresh-start 2/3/4/6 stages as their default path.

#### Scenario: Gate summaries identify evidence and failures
- **WHEN** any MNT-03 gate completes
- **THEN** it writes a machine-readable summary with pass/fail counts
- **AND** it points to artifacts for topology, task reports, daemon logs, broker logs, pcap, conntrack, tc, cleanup, and the checkpoint reached
