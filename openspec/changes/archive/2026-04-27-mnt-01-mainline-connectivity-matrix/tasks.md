## 1. Case Model And Fixture Contract

- [x] 1.1 Define the MNT-01 case/profile model for UDP profile, TCP profile, IP/profile modifiers, helper modifiers, link modifiers, direction, expected outcome, and required evidence.
- [x] 1.2 Add a mainline two-peer fixture that runs real `miopunch` peer processes with self-hosted MQTT signaling and optional STUN.
- [x] 1.3 Enforce fixture boundaries so the fixture may inject only identity, peer config, hello/auth bootstrap, MQTT/STUN endpoints, test ports, and network profile data.
- [x] 1.4 Add fixture artifacts that record injected setup material and prove no NAT result, candidate path, selected path, neighbor state, success cache, or payload result was preloaded.

## 2. Matrix Coverage

- [x] 2.1 Implement UDP profile coverage for `udp-nat1`, `udp-nat2`, `udp-nat3`, `udp-nat4-regular`, and `udp-nat4-irregular`.
- [x] 2.2 Generate the UDP unordered 15-class matrix for the full gate.
- [x] 2.3 Implement TCP profile coverage for `tcp6-direct`, `tcp4-direct`, `tcp4-portmap-direct`, `tcp-easy-stable`, `tcp-hard-regular`, `tcp-hard-irregular`, and `tcp-blocked-unknown`.
- [x] 2.4 Generate the TCP directed 49-class matrix and preserve `dialer -> target` direction in case identity and reports.
- [x] 2.5 Add specialty coverage for `auto` priority, IPv6 fallback, portmap helper behavior, loss/netem, blocked paths, STUN unavailable, and representative transport variants without multiplying them into the primary matrices.

## 3. Evidence And Outcome Enforcement

- [x] 3.1 Add outcome classification handling for `success-required`, `success-preferred`, `diag-fail-allowed`, and `fail-required`.
- [x] 3.2 Require MQTT signaling, candidate discovery, attempt path, and selected/failed path evidence for every MNT-01 case.
- [x] 3.3 Require payload exchange evidence for successful cases and failure stage, reason, and stop condition evidence for failed cases.
- [x] 3.4 Add broker log/pcap checks that prove MQTT is not carrying data-plane payload.
- [x] 3.5 Add bounded repeat/retry reporting for TCP hard/irregular cases, including budget, attempt counts, observed result summary, and consistent failure reasons.

## 4. Gate Entry Points And Reports

- [x] 4.1 Add an MNT-01 smoke gate covering MQTT-only signaling, representative direct paths, punching paths, TCP hard diagnostics, and `auto` path priority.
- [x] 4.2 Add an MNT-01 selftest gate covering the UDP 15-class matrix and representative TCP risk cases.
- [x] 4.3 Add an MNT-01 fulltest gate covering the UDP 15-class matrix and TCP 49-class directed matrix.
- [x] 4.4 Add aggregate reports that summarize required passes, preferred successes, allowed diagnostic failures, required failures, unexpected failures, and artifact locations.
- [x] 4.5 Record non-test product issues discovered during implementation or execution in `docs/notes/mainline-network-test-findings.md` instead of mixing fixes into MNT-01.

## 5. Documentation And Validation

- [x] 5.1 Update relevant lab/runbook documentation with MNT-01 gate usage, scope boundaries, and artifact locations.
- [x] 5.2 Verify OpenSpec status for `mnt-01-mainline-connectivity-matrix` before implementation starts.
- [x] 5.3 During apply, run focused validation after each gate is added.
- [x] 5.4 Before merging code-affecting implementation, run the `$dev` required host and lab validation gates or document any blocked gate with evidence.

## 6. Verification Follow-ups

- [x] 6.1 Disclose MNT-01 hello/auth bootstrap in fixture artifacts and OpenSpec/docs.
- [x] 6.2 Add representative specialty gate cases for STUN unavailable, IPv6 fallback, loss/netem, and transport variants.
- [x] 6.3 Enforce attempt evidence and stop-condition reporting for diagnostic and required-failure outcomes.
- [x] 6.4 Aggregate required, preferred, diagnostic, required-failure, and unexpected outcome counts in gate summaries.
- [x] 6.5 Make `miopunch up --localapi` lab overrides independent of default LocalAPI daemon probes.
