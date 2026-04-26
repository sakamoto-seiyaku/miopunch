## ADDED Requirements

### Requirement: MNT-01 uses real mainline peers with bounded fixture injection
The MNT-01 connectivity matrix SHALL use real `miopunch` mainline peer processes as the connectivity subject.

The test fixture SHALL be allowed to provide only minimum setup material: identity, peer identity/config, hello/auth bootstrap material, self-hosted MQTT endpoint, STUN endpoint, local network profile, and test ports.

Hello/auth bootstrap material MAY include a governance head snapshot and member approval declaration only to satisfy the existing mainline hello handshake. This bootstrap material MUST be disclosed in fixture artifacts as `auth_bootstrap` and MUST NOT be treated as coverage of invite, approve, join, governance, decl synchronization, or membership behavior.

The test fixture MUST NOT pre-populate NAT classification results, candidate path conclusions, selected attempt paths, active neighbor state, success cache, or payload results.

#### Scenario: Fixture does not inject connectivity conclusions
- **WHEN** an MNT-01 case starts two peers
- **THEN** the peers obtain candidates and attempt paths through the normal mainline connectivity flow
- **AND** fixture artifacts disclose any hello/auth bootstrap material
- **AND** the fixture state does not contain precomputed NAT results or selected path results

### Requirement: MNT-01 uses self-hosted MQTT as the only required signaling path
The MNT-01 required gates SHALL use a test-environment self-hosted MQTT broker for signaling.

The MNT-01 required gates MUST NOT use `coord` as a mainline signaling path and MUST NOT treat `coord` as fallback.

Public MQTT brokers SHALL NOT be required for MNT-01 required gates.

#### Scenario: Required case records MQTT signaling evidence
- **WHEN** an MNT-01 required case runs
- **THEN** the case uses the self-hosted MQTT broker for signaling
- **AND** artifacts include MQTT signaling evidence
- **AND** the case does not connect to `coord`

### Requirement: UDP matrix covers five profiles as unordered pair classes
The MNT-01 UDP matrix SHALL cover these UDP profiles:
`udp-nat1`, `udp-nat2`, `udp-nat3`, `udp-nat4-regular`, and `udp-nat4-irregular`.

The full UDP matrix SHALL cover unordered pair classes with replacement for these profiles, for a total of 15 pair classes.

#### Scenario: Full UDP matrix includes all unordered pairs
- **WHEN** the MNT-01 full gate enumerates UDP cases
- **THEN** every unordered pair of the five UDP profiles is represented exactly as a matrix class

### Requirement: TCP matrix covers seven profiles as directed pair classes
The MNT-01 TCP matrix SHALL cover these TCP profiles:
`tcp6-direct`, `tcp4-direct`, `tcp4-portmap-direct`, `tcp-easy-stable`, `tcp-hard-regular`, `tcp-hard-irregular`, and `tcp-blocked-unknown`.

The full TCP matrix SHALL cover directed `dialer -> target` pair classes for these profiles, for a total of 49 directed pair classes.

The reverse direction MUST be represented as a separate class when it is required by the matrix.

#### Scenario: Full TCP matrix preserves direction
- **WHEN** the MNT-01 full gate enumerates TCP cases
- **THEN** each ordered `dialer -> target` profile pair is represented
- **AND** `A -> B` is not treated as equivalent to `B -> A`

### Requirement: Specialty axes remain outside the primary matrix
MNT-01 SHALL treat `auto`, IPv6 fallback, portmap helper behavior, loss/netem, blocked paths, STUN unavailable, and transport variants as specialty coverage outside the primary UDP/TCP matrix.

MNT-01 MUST NOT require the Cartesian product of all specialty axes with the primary UDP/TCP matrices.

MNT-01 smoke or selftest gates SHALL include representative specialty cases for `auto`, IPv6 fallback, portmap helper behavior, loss/netem, blocked paths, STUN unavailable, and transport variants.

When `p2p_network=auto` is tested, the expected attempt priority SHALL be `tcp6 -> tcp4 -> udp6 -> udp4`.

#### Scenario: Auto specialty validates attempt priority
- **WHEN** an MNT-01 auto specialty case runs
- **THEN** artifacts show TCP paths are attempted before UDP paths
- **AND** the case is not counted as part of the UDP 15-class or TCP 49-class full matrix

### Requirement: Cases declare outcome classification
Every MNT-01 case SHALL declare one outcome classification:
`success-required`, `success-preferred`, `diag-fail-allowed`, or `fail-required`.

For `success-required`, the case SHALL fail if payload exchange is not proven.

For `success-preferred`, the case SHOULD prefer success, but a TCP hard failure may pass only when diagnostics are complete and the failure is within the declared budget.

For `diag-fail-allowed`, the case SHALL NOT require connectivity success, but SHALL require correct attempt evidence and explainable failure.

For `fail-required`, the case SHALL fail if connectivity unexpectedly succeeds.

#### Scenario: Diagnostic failure is accepted only with evidence
- **WHEN** an MNT-01 case classified as `diag-fail-allowed` fails to connect
- **THEN** the gate passes only if artifacts include attempted path, failure stage, reason, and stop condition

### Requirement: TCP hard cases use bounded repeat and diagnostics
MNT-01 TCP hard or irregular cases SHALL use bounded repeat/retry semantics when the case is classified as `success-preferred` or `diag-fail-allowed`.

The case report SHALL include attempt budget, repeat count or retry count, observed successes or failures, and consistent failure reasons when failures occur.

#### Scenario: TCP hard case reports bounded observations
- **WHEN** a TCP hard/irregular MNT-01 case completes
- **THEN** the report includes the configured budget and observed result summary
- **AND** failures include explainable reasons instead of silent timeouts

### Requirement: Artifacts prove success or explain failure
Every MNT-01 case SHALL collect artifacts sufficient to prove the outcome:
MQTT signaling evidence, candidate discovery evidence, attempt path evidence, final selected or failed path, peer logs, broker logs, and network artifacts.

Successful cases SHALL include payload exchange evidence.

Failed cases SHALL include failure stage, reason, and stop condition.

The stop condition SHALL be recorded in per-attempt artifacts and the per-case summary.

Broker logs or pcap MUST NOT show data-plane payload relay through MQTT.

#### Scenario: Success requires payload evidence
- **WHEN** an MNT-01 case exits successfully
- **THEN** artifacts include payload exchange evidence
- **AND** broker artifacts do not contain the data-plane payload

### Requirement: MNT-01 provides layered gates
MNT-01 SHALL provide layered gates for different cost levels:
smoke, selftest, and fulltest.

The smoke gate SHALL cover representative MQTT-only signaling, path-priority, STUN unavailable, and transport variant cases.

The selftest gate SHALL cover the full UDP 15-class matrix, representative TCP risk cases, and representative IPv6 fallback plus loss/netem specialty cases.

The fulltest gate SHALL cover the UDP 15-class matrix and the TCP 49-class directed matrix.

#### Scenario: Fulltest covers complete primary matrices
- **WHEN** the MNT-01 fulltest gate runs
- **THEN** it executes the full UDP primary matrix
- **AND** it executes the full TCP primary matrix
- **AND** the final report summarizes required passes, preferred successes, allowed diagnostic failures, required failures, and unexpected failures

### Requirement: Product issues discovered by MNT-01 are recorded separately
When MNT-01 implementation or execution discovers a project code issue that is not caused by the test harness itself, the issue SHALL be recorded in `docs/notes/mainline-network-test-findings.md`.

MNT-01 implementation MUST NOT silently fix unrelated product issues inside the test change.

#### Scenario: Product bug is discovered during matrix implementation
- **WHEN** MNT-01 execution exposes a non-test product defect
- **THEN** the defect is recorded in the findings file with reproduction conditions and evidence
- **AND** the test change does not include an unrelated product fix
