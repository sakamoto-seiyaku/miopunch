## ADDED Requirements

### Requirement: MNT-03 uses real product nodes in controlled NAT domains
MNT-03 SHALL run real `miopunch` product nodes as Docker/systemd Linux instances.

The lab SHALL provide only infrastructure: NAT domains, WAN, MQTT broker, STUN/probe services, packet capture, conntrack snapshots, netem, and perturbation controls. The lab MAY write audited infrastructure fixture fields required to bind a real product node to that environment, such as MQTT/STUN/P2P port and IP-family settings. The lab MUST NOT inject membership, peer creation, governance, decls, bootstrap candidates, active neighbors, reachability decisions, selected paths, payload success, or recovered topology state.

Docker SHALL be used for node lifecycle and systemd isolation. The harness SHALL attach each node container network namespace to a lab-controlled NAT domain via veth or equivalent network-namespace wiring, so Docker default bridge networking does not bypass the NAT fixture.

#### Scenario: Node starts blank behind lab NAT
- **WHEN** an MNT-03 case starts node `n01`
- **THEN** `n01` runs a real `miopunch` system daemon
- **AND** the lab provides only its network environment and control-plane endpoints
- **AND** `n01` has no pre-populated membership, peer, governance, decl, bootstrap, or neighbor state
- **AND** any lab-written local fixture fields are reported separately from product semantic state

### Requirement: MNT-03 validates 12-node blank formation and join
MNT-03 SHALL use the 12-node profile defined by `docs/decisions/mainline-network-test-charter.md`.

Nodes `n01..n11` SHALL form one network through real `up -> invite -> approve -> join` product flows. Node `n12` SHALL be reserved for actor and lifecycle validation and SHALL NOT be counted in stable-topology pass-rate statistics.

#### Scenario: Legal nodes join the same network
- **WHEN** `n01` creates the network and approves `n02..n11`
- **THEN** `n02..n11` join through the real product join flow
- **AND** all legal nodes report the same `net_id`, governance head, and decls head
- **AND** artifacts show the invite, approve, join, and state snapshots for each legal node

### Requirement: MNT-03 implementation validates incremental real-node milestones
MNT-03 implementation SHALL proceed through incremental real-node milestones before the complete 12-node fulltest is considered valid:
`2-node substrate -> 3-node bootstrap -> 4-node reachability/portmap -> 6-node bootstrap_more/hard carry -> 12-node full topology`.

Each milestone SHALL run in the QEMU lab VM with real Docker/systemd `miopunch` nodes and lab-controlled NAT fixtures. A later milestone MUST NOT be treated as complete until the previous milestone has produced a passing machine-readable summary and artifacts.

#### Scenario: Milestone blocks later expansion until verified
- **WHEN** the 4-node reachability milestone has not produced a passing summary
- **THEN** the MNT-03 implementation does not treat the 6-node bootstrap_more milestone as complete
- **AND** artifacts identify the failing milestone and its evidence

### Requirement: MNT-03 validates bootstrap recommendations and bootstrap_more
MNT-03 SHALL validate that mainline nodes select bootstrap candidates from product state, not lab state.

Initial bootstrap recommendations SHALL contain two candidates when enough eligible peers exist. If initial candidates fail, the joiner SHALL request more candidates through `bootstrap_more`. The system SHALL perform at most two `bootstrap_more` rounds and SHALL return two new de-duplicated candidates per successful round when enough eligible peers remain.

Candidate selection SHALL prefer `direct/easy` reachability buckets and then progressively relax to `hard1`, `hard2`, and `unknown`. When only some selected peers have local dial fixtures or peer config, locally dialable candidates MUST be attempted before report-only non-dialable candidates. Attempts MUST be bounded and MUST report selected bucket, attempted peer IDs, rejected duplicates, and failure reasons.

#### Scenario: Joiner obtains more bootstrap candidates after failures
- **WHEN** a joiner exhausts its initial bootstrap candidates
- **THEN** it sends a `bootstrap_more_request`
- **AND** it receives de-duplicated candidates from the next eligible bucket
- **AND** the final report includes attempted candidates, selected buckets, and stop condition

### Requirement: MNT-03 validates presence and reachability hints
MNT-03 SHALL validate product presence and reachability hints.

Each joined node SHALL report `v4_hint` and `v6_hint` without exposing endpoint addresses in the hint. Presence SHALL carry enough state-head evidence to detect governance or decl divergence. Presence and hints SHALL be used only for ordering and maintenance decisions; they MUST NOT be treated as connectivity proof.

#### Scenario: Reachability hints are visible but not endpoint leaks
- **WHEN** an MNT-03 selftest collects topology snapshots
- **THEN** each joined node exposes `v4_hint` and `v6_hint`
- **AND** the hint values do not include IP addresses or ports
- **AND** presence evidence includes state-head information

### Requirement: MNT-03 validates logN active neighbor maintenance
After network formation, mainline nodes SHALL maintain active neighbors according to `k=max(2,ceil(ln(n)))`, where `n` is the known legal peer count.

For the default 12-node profile, stable nodes SHALL target approximately three active neighbors. Hard nodes SHALL have at least one active `direct/easy` neighbor when such a neighbor is reachable. Admin nodes MUST NOT become the sole hub for all active edges.

#### Scenario: Stable topology reaches the target neighbor shape
- **WHEN** the MNT-03 selftest waits for topology stabilization
- **THEN** stable legal nodes report active neighbors near the target `k`
- **AND** hard nodes are carried by at least one easy/direct active neighbor where reachable
- **AND** the degree distribution shows admin nodes are not the only hub

### Requirement: MNT-03 validates active-edge data-plane evidence
MNT-03 SHALL validate active neighbor edges through `ping` or an equivalent minimal payload exchange.

Successful active edges SHALL report selected `attempt_path`, `data_proto`, and payload evidence. Failed active edges SHALL be accepted only when the node profile declares the edge explainably hard or unknown, and the report includes stage, reason, attempt budget, and stop condition.

MQTT broker logs and packet captures MUST NOT contain data-plane payload relay.

#### Scenario: Active edge success includes payload evidence
- **WHEN** an active neighbor edge succeeds
- **THEN** the edge report includes `attempt_path`, `data_proto`, and payload evidence
- **AND** broker artifacts show MQTT was not used as a data-plane relay

### Requirement: MNT-03 validates perturbation and recovery
MNT-03 fulltest SHALL inject perturbations only after the stable topology has passed.

Fulltest SHALL cover loss/netem, node offline, rejoin, revoke, IPv6 blocking, portmap blocking, and broker outage/recovery. The system SHALL report neighbor replacement or reconnect attempts, non-fault node health, revoked-node access denial, and rejoin convergence or explainable failure.

#### Scenario: Rejoin recovers or fails explainably
- **WHEN** a lifecycle node goes offline and later rejoins
- **THEN** the system either restores active neighbors and payload-capable edges
- **OR** it reports a bounded, explainable recovery failure
- **AND** artifacts include before/after topology diff and recovery evidence

### Requirement: MNT-03 provides layered gates
The lab host SHALL expose `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest`.

`mnt03-smoke` SHALL run the 2-node substrate milestone and the 3-node bootstrap milestone, including blank startup, real join, topology snapshots, and at least one successful payload edge.

`mnt03-selftest` SHALL add the 4-node reachability/portmap milestone and the 6-node bootstrap_more/hard carry milestone, including presence, reachability bucket, logN active neighbor, hard-node carry, admin hub avoidance, and active-edge evidence.

`mnt03-fulltest` SHALL run the complete 12-node topology and perturbation/recovery validation, including packet capture, conntrack, NAT rules, tc qdisc statistics, broker logs, daemon journals, task reports, topology snapshots, and cleanup logs.

#### Scenario: Gate summaries identify evidence and failures
- **WHEN** any MNT-03 gate completes
- **THEN** it writes a machine-readable summary with pass/fail counts
- **AND** it points to artifacts for topology, task reports, daemon logs, broker logs, pcap, conntrack, tc, and cleanup
