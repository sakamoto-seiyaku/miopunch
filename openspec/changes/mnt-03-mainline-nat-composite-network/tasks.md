## 1. Charter and OpenSpec

- [x] 1.1 Update `docs/decisions/mainline-network-test-charter.md` to define MNT-03 as Docker/systemd product nodes attached to lab-controlled NAT domains
- [x] 1.2 Add proposal, specs, design, and task artifacts for `mnt-03-mainline-nat-composite-network`
- [x] 1.3 Run `openspec validate mnt-03-mainline-nat-composite-network`
- [x] 1.4 Rework design, specs, and tasks for incremental 2/3/4/6/12-node implementation

## 2. Shared Substrate and Observability

- [ ] 2.1 Add a mainline topology snapshot model covering local identity, net/head summaries, reachability hints, presence, bootstrap, active neighbors, degree distribution, latest attempts, payload evidence, and recovery evidence
- [ ] 2.2 Add `GET /api/v0/topology` using the topology snapshot model
- [ ] 2.3 Add CLI JSON access to the same topology evidence without reading private state files directly from tests
- [ ] 2.4 Add redaction tests proving topology output excludes secrets, invite secrets, raw private keys, and unredacted join codes
- [ ] 2.5 Add reusable guest helpers for Docker/systemd nodes attached to lab NAT domains via container netns + veth
- [ ] 2.6 Add shared artifact collection for command output, task reports, daemon journals, topology snapshots, broker logs, network manifests, cleanup logs, and machine-readable summaries
- [ ] 2.7 Add host `labctl` entries and guest runner plumbing for stage-based MNT-03 execution without marking any stage complete yet

## 3. M1: 2-Node Substrate

- [ ] 3.1 Define the `2node-substrate` profile with `n01` primary admin and `n03` easy/direct member
- [ ] 3.2 Start two Docker/systemd product nodes behind lab-controlled easy/direct NAT domains with self-hosted MQTT and optional STUN
- [ ] 3.3 Run blank `up`, real `invite`, `approve`, `join`, topology snapshot, and `ping` payload exchange through product CLI/LocalAPI
- [ ] 3.4 Prove lab did not inject membership, peers, governance, decls, bootstrap, neighbor, or payload success state
- [ ] 3.5 Run the 2-node stage in the QEMU lab VM and save artifacts before proceeding
- [ ] 3.6 Fix or record any product defects found by the 2-node stage before starting M2

## 4. M2: 3-Node Bootstrap

- [ ] 4.1 Define the `3node-bootstrap` profile by adding `n04` as a second easy member
- [ ] 4.2 Implement or expose initial bootstrap recommendation evidence with two candidates when enough eligible peers exist
- [ ] 4.3 Implement minimal signed presence with governance and decls head summaries needed for online/state-head evidence
- [ ] 4.4 Validate multi-member consistency across `n01`, `n03`, and `n04`
- [ ] 4.5 Run the 3-node stage in the QEMU lab VM and save bootstrap, presence, topology, and payload artifacts
- [ ] 4.6 Fix or record any product defects found by the 3-node stage before starting M3

## 5. M3: 4-Node Reachability and Portmap

- [ ] 5.1 Define the `4node-reachability` profile by adding `n05` as NAT3 + NAT-PMP/portmap member
- [ ] 5.2 Persist and expose `v4_hint` and `v6_hint` for joined peers with the defined hint ordering
- [ ] 5.3 Validate reachability bucket ordering and portmap candidate evidence through topology diagnostics
- [ ] 5.4 Implement target neighbor count `k=max(2,ceil(ln(n)))`; for 4 nodes the target remains 2
- [ ] 5.5 Validate active neighbor list, degree distribution, selected attempt path, data protocol, and payload evidence
- [ ] 5.6 Run the 4-node stage in the QEMU lab VM and save reachability, portmap, topology, and payload artifacts
- [ ] 5.7 Fix or record any product defects found by the 4-node stage before starting M4

## 6. M4: 6-Node Bootstrap More and Hard Carry

- [ ] 6.1 Define the `6node-bootstrap-more` profile by adding `n06` dual-stack fallback and `n07` hard-regular member
- [ ] 6.2 Implement `bootstrap_more_request` and `bootstrap_more_response` over the control-plane mailbox
- [ ] 6.3 Enforce bounded bootstrap_more semantics: 5s timeout, at most two rounds, and up to two de-duplicated candidates per response
- [ ] 6.4 Add unit tests for candidate de-duplication, bucket ordering, timeout, and exhausted-candidate reports
- [ ] 6.5 Validate IPv6-first fallback and hard-node carry by easy/direct active neighbors
- [ ] 6.6 Prevent stable topology from selecting only the primary admin when enough non-admin candidates exist
- [ ] 6.7 Run the 6-node stage in the QEMU lab VM and save bootstrap_more, fallback, hard-node, topology, and payload artifacts
- [ ] 6.8 Fix or record any product defects found by the 6-node stage before starting M5

## 7. M5: 12-Node Full Topology

- [ ] 7.1 Define the complete 12-node profile from the charter, including NAT1/NAT2/NAT3/NAT4, IPv6, portmap, hard/irregular, lossy, and lifecycle roles
- [ ] 7.2 Implement reachability-bucket neighbor selection with random or rotating choice within a bucket
- [ ] 7.3 Add neighbor health monitoring from data-plane activity, keepalive, or equivalent product evidence
- [ ] 7.4 Implement bounded reconnect and replacement when an active neighbor becomes unhealthy
- [ ] 7.5 Expose active neighbor list, degree distribution, reconnect attempts, replacement evidence, and hard/unknown explainable failures through topology diagnostics
- [ ] 7.6 Add unit tests for neighbor target count, bucket selection, admin hub avoidance, unhealthy replacement, and hard-node carry
- [ ] 7.7 Run the 12-node stable-topology stage in the QEMU lab VM and save topology, degree, active-edge, and payload/failure artifacts
- [ ] 7.8 Keep `n12` as actor/lifecycle and exclude it from stable-topology pass-rate statistics

## 8. M6: Perturbation and Recovery

- [ ] 8.1 Add recovery evidence for node offline, rejoin, broker outage/recovery, IPv6 block, portmap block, and loss/netem perturbations
- [ ] 8.2 Ensure revoke prevents further protected access while non-revoked peers remain usable
- [ ] 8.3 Ensure recovery failures report stage, reason code, contacted peers, retry budget, and stop condition
- [ ] 8.4 Add task/report tests for revoke, rejoin, broker outage/recovery, and explainable recovery failure
- [ ] 8.5 Run the 12-node perturbation stage in the QEMU lab VM with pcap, conntrack, NAT rules, and tc qdisc evidence
- [ ] 8.6 Fix or record any product defects found by the perturbation stage before final validation

## 9. Public Gates and Final Validation

- [ ] 9.1 Implement `mnt03-smoke` as the public gate for M1 and M2
- [ ] 9.2 Implement `mnt03-selftest` as the public gate for M1 through M4
- [ ] 9.3 Implement `mnt03-fulltest` as the public gate for M1 through M6
- [ ] 9.4 Ensure all gates write machine-readable summaries with pass/fail counts and artifact paths
- [ ] 9.5 Ensure artifacts are pulled into `lab/_artifacts/` for success and failure cases
- [ ] 9.6 Run `go test ./...`
- [ ] 9.7 Run `go vet ./...`
- [ ] 9.8 Run `bash scripts/check_no_xtcp_imports.sh`
- [ ] 9.9 Run `./lab/host/labctl mnt03-smoke`
- [ ] 9.10 Run `./lab/host/labctl mnt03-selftest`
- [ ] 9.11 Run `./lab/host/labctl mnt03-fulltest`
- [ ] 9.12 Record any product defects discovered during implementation in `docs/notes/mainline-network-test-findings.md` unless they are required MNT-03 behavior
