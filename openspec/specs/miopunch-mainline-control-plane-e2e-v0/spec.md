# miopunch-mainline-control-plane-e2e-v0 Specification

## Purpose
TBD - created by archiving change mnt-02-mainline-control-plane-e2e. Update Purpose after archive.
## Requirements
### Requirement: MNT-02 uses real mainline peers with bounded fixture injection
The MNT-02 control-plane e2e gate SHALL use real `miopunch` mainline daemon processes as the system under test.

The test fixture SHALL be allowed to provide only minimum setup material: network topology, per-node state paths, self-hosted MQTT endpoint, optional STUN endpoint, and test ports.

The test fixture MUST NOT pre-populate membership, decls, peer lists, invite approval outcomes, task results, or recovered control-plane state that would otherwise be produced by `invite/approve/join` flows.

#### Scenario: Fixture starts from blank control-plane state
- **WHEN** an MNT-02 case starts N peers
- **THEN** membership state is created only via `miopunch invite/approve/join`
- **AND** fixture artifacts disclose any seeded identity material
- **AND** fixture artifacts do not contain pre-created approvals or pre-populated peer state

### Requirement: MNT-02 uses self-hosted MQTT as the only required signaling path
The MNT-02 required gates SHALL use a test-environment self-hosted MQTT broker for signaling.

The MNT-02 required gates MUST NOT use `coord` as a signaling path and MUST NOT treat `coord` as fallback.

Public MQTT brokers SHALL NOT be required for MNT-02 required gates.

#### Scenario: Required case records MQTT signaling evidence
- **WHEN** an MNT-02 required case runs
- **THEN** the case uses the self-hosted MQTT broker for signaling
- **AND** artifacts include broker logs and/or pcap evidence
- **AND** the case does not connect to `coord` or public brokers

### Requirement: MNT-02 validates invite/approve/join closed loop
MNT-02 SHALL validate the control-plane closed loop from blank nodes:
`up -> invite -> approve -> join`.

The case artifacts SHALL include the invite code (redacted), join request evidence, membership bundle evidence, and post-join state snapshots.

#### Scenario: A joiner becomes a member via approval
- **WHEN** an issuer peer runs `miopunch invite`
- **AND** the issuer peer runs `miopunch approve <invite_code>`
- **AND** a joiner peer runs `miopunch join <invite_code>`
- **THEN** the joiner receives a membership bundle within the invite expiry
- **AND** the joiner persists net/governance/decl state
- **AND** the joiner can list at least the seed peer(s) via `miopunch ls`

### Requirement: MNT-02 validates multi-member consistency
MNT-02 selftest gates SHALL validate multi-member join and consistency across at least 3 member peers.

#### Scenario: Multiple members join and can discover peers
- **WHEN** an MNT-02 selftest case joins at least 3 members into the same net
- **THEN** each member can list peers via `miopunch ls`
- **AND** peer IDs shown in `ls` are consistent across members within a bounded stabilization window

### Requirement: MNT-02 validates minimal data-plane usability after join
After a successful join, MNT-02 SHALL validate that members can exchange a minimal payload using `miopunch ping`.

#### Scenario: Joined member can ping another member
- **WHEN** two members have completed `join`
- **THEN** `miopunch ping <peer_id>` completes successfully
- **AND** artifacts include `ping=ok` evidence in the task report

### Requirement: MNT-02 includes a shell smoke case
MNT-02 smoke gates SHALL include a shell smoke case to validate the end-to-end product workflow surface.

#### Scenario: Shell list succeeds after join
- **WHEN** a member has completed `join`
- **THEN** `miopunch sh ls <peer_id>` completes successfully
- **AND** artifacts include the task report and redacted output

### Requirement: MNT-02 validates idempotency of core control-plane tasks
MNT-02 selftest gates SHALL validate idempotency for core tasks:
`up`, `invite`, `approve`, `join`, `ls`, and `ping`.

#### Scenario: Repeating join is stable
- **WHEN** a joiner runs `miopunch join <invite_code>` twice
- **THEN** the second run terminates with a stable, explainable outcome
- **AND** artifacts include stage and reason_code evidence for both runs

### Requirement: MNT-02 validates broker outage and recovery
MNT-02 selftest gates SHALL validate that broker outage results in explainable failure, and broker recovery restores forward progress.

#### Scenario: Broker outage is explainable and recovery resumes progress
- **WHEN** the broker becomes unavailable during a control-plane operation
- **THEN** the operation fails with an explainable stage and reason_code
- **AND** after broker recovery, the same operation can be retried and completes successfully

### Requirement: MNT-02 validates revoke boundary
MNT-02 selftest gates SHALL validate that revocation prevents further access by the revoked peer without breaking unrelated peers.

#### Scenario: Revoked peer cannot continue to use the net
- **WHEN** an issuer revokes a member via `miopunch revoke <peer_id> --dangerous`
- **THEN** subsequent `ping` operations by the revoked peer fail with an explainable outcome
- **AND** a non-revoked member can still `ping` the issuer successfully

### Requirement: MNT-02 enforces report export and redaction
Every MNT-02 case SHALL export task reports and SHALL enable redaction.

Reports MUST NOT include secrets such as `secret_key`, `net_secret_b64`, `invite_secret_b64`, or unredacted invite codes.

#### Scenario: Report export redacts secrets
- **WHEN** an MNT-02 case exports a report with `--redact`
- **THEN** the report does not contain unredacted secret material

### Requirement: MNT-02 provides layered gates
MNT-02 SHALL provide layered gates for different cost levels, at minimum:
smoke and selftest.

The smoke gate SHALL validate the minimal control-plane closed loop plus minimal payload/shell smoke.

The selftest gate SHALL validate multi-member consistency, idempotency, broker outage/recovery, and revoke boundary behavior.

#### Scenario: Smoke and selftest boundaries are explicit
- **WHEN** MNT-02 gates are executed
- **THEN** smoke runs the minimal required cases
- **AND** selftest runs the extended coverage set
- **AND** the final report summarizes passed and failed cases with pointers to artifacts

