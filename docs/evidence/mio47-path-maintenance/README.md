# MIOPUNCH-47 evidence

Behavior and boundaries are recorded in
[the connection-maintenance note](../../notes/2026-10-02-connection-path-maintenance.md).
The rerunnable entry is `scripts/check_path_maintenance.py`: a new output
directory is required; default runs the ordered matrix, `--case NAME` runs one
case, `--suite` adds affected short regression, and `--suite --start-at NAME`
resumes a failed short group without repeating already passed acceptance.
Each receipt preserves the actual command, exit, raw log and owned cleanup.
Named acceptance must exist and exactly one test must execute.

## Final runtime and affected checks

- [final-fresh-ack-acceptance/results.json](final-fresh-ack-acceptance/results.json)
  is the authoritative final fourteen-case matrix after independent review,
  including the unchanged-address physical outage and fresh original-path ACK.
  It retains healthy same-address success, Retryable during actual loss and
  Terminal after the original lifetime ends.
- [fresh-ack-protocol-checks/results.json](fresh-ack-protocol-checks/results.json)
  records the packet-number watermark regression and 45 affected vendored
  multipath tests. [fresh-ack-immediate-close-check/run.log](fresh-ack-immediate-close-check/run.log)
  adds immediate abandoned-path and closed-Connection rejection before drain.
- [final-fresh-ack-checks/results.json](final-fresh-ack-checks/results.json)
  records the final library, all-targets, WASM and extended root/fork formatting
  checks. The earlier affected short matrix below remains applicable: the
  additive probe reads existing authenticated ACK state and queues an ordinary
  PING; packet encoding, ACK processing and the NAT algorithm are unchanged.
- [final-roles-acceptance/results.json](final-roles-acceptance/results.json)
  preserves the earlier thirteen-case ordered matrix, including both
  original endpoint roles in protected no-backup recovery. Tests use real
  independent identities/SQLite, TLS/SCRAM, streams and physical socket faults.
- [final-suite/results.json](final-suite/results.json) retains the first full
  attempt: twelve initial acceptance cases, 26 resources and 38 native
  gate/proof/revocation short tests passed, then a legacy MQTT CLI close check
  failed. It is deliberately not reported as a passing suite.
- [continued-short/results.json](continued-short/results.json) resumes at the
  genuine failed group after the native-gated compatibility fix. It reports
  120 additional passing short tests: MQTT 19, node-state 38, TCP/member
  forwarding 20, physical bindings 12, formal node budget 6, library 25.
  All-targets/current hooks, WASM compile and root/fork formatting passed.
  Combined affected short coverage is 184 reported passing entries, including
  the existing inert subprocess worker entries; this is not a new NAT success
  probability or a claim that each worker entry independently ran a lab.
- [final-role-checks/results.json](final-role-checks/results.json) is the
  final private-envelope/role-delegation type, WASM and formatting check.
  That SDK adjustment changed no previously passed transport, selector,
  persistence, admission, management or NAT traversal algorithm.
- [gated-final-acceptance/results.json](gated-final-acceptance/results.json)
  preserves the twelve-case intermediate compatibility check before adding
  the original-accepting-side case. It is superseded by thirteen-case final
  acceptance above.

Already verified unchanged slow NAT matrices are skipped under the user's exact
policy, while actual physical binding and affected formal-budget short tests
remain. No root-imposed executable freezes, duplicate full source archives,
all-history SHA audits or additional root gates were used.

## Genuine behavior RED/GREEN

| Behavior | RED | GREEN |
| --- | --- | --- |
| Add UDP to an original admitted TCP business Connection | [first-red](first-red/results.json) rejected ordinary connection | [first-green](first-green/results.json) same stream and both stable IDs |
| Reevaluate real RTT change without replacing either living path | [quality-controlled-red](quality-controlled-red/results.json) kept slower route despite better actual RTT | [quality-green](quality-green/results.json) selected faster path with unchanged path IDs |
| Protected control beside business quality selection | [isolation-red](isolation-red/results.json) diverted explicit TCP business attempt into another MQTT control route | [isolation-compile-corrected](isolation-compile-corrected/results.json) independently selected qualified business paths, signal path rejected |
| Keep repair control alive while original business path validates | [control-recovery-green](control-recovery-green/results.json) actual early-close/read failure | [control-recovery-owned-lifetime](control-recovery-owned-lifetime/results.json) physical TCP cut repaired on same stream/IDs |
| Retained same-address path during actual TCP outage | [same-address-red](same-address-red/results.json) falsely reported recovery from the stale list entry | [same-address-green](same-address-green/results.json) requires fresh original-path ACK, then Retryable/Terminal |
| Initiate repair from original accepting side | [accepting-side-red](accepting-side-red/results.json) real timeout because only noq dialing side can open paths | [accepting-side-green](accepting-side-green/results.json) delegated to original dialing peer on same stream/IDs |

Each final acceptance rechecks the resulting behavior on the final runtime.
The priority failure check ran immediately after first GREEN, before quality
conditions were introduced; [priority-green](priority-green/results.json)
retains the old stream failure and distinct retained/new handle IDs.

## Build, fixture and compatibility diagnosis

- `quality-red` and `quality-fixture-diagnosis` are fixture failures before the
  intended RTT change: the previously admitted TCP route replaced the controlled
  proxy contact. Rebinding the admitted endpoints isolated both actual paths;
  `quality-controlled-red` is the meaningful quality RED.
- `isolation-green` preserves a Rust borrow error before tests ran. The corrected
  compile and actual runtime are separate in `isolation-compile-corrected`.
- `control-recovery-api-build` records missing new public SDK APIs before their
  implementation. It is a compile failure, not a behavioral RED.
- The first full affected run exposed the unchanged old CLI's original
  three-second server-close assertion. [mqtt-close-repro](mqtt-close-repro/results.json)
  reproduced the failure three times. `mqtt-close-instrumentation` showed that
  the legitimate Connection received `done` and closed, while Router shutdown
  remained pending; the problem was not failed application delivery.
- `mqtt-close-bootstrap-hint` and `mqtt-close-owned-failure-endpoint` are
  disproved narrow runtime probes. `mqtt-close-baseline-selection-probe` also
  failed with the old selector/application functions. All probe edits were
  removed. `src/main.rs` and `tests/mqtt_probe.rs` are unchanged from baseline;
  the old timeout was not relaxed.
- [mqtt-close-exact-baseline](mqtt-close-exact-baseline/results.json) passed
  after temporarily restoring only the eight owned runtime diff files to the
  supplied pre-47 HEAD `56d40e0ee07dee6a2ed0ec6a9115ed26c59be60d`. User WIP and
  shared build configuration were untouched; the complete47 edits were restored
  after the comparison. This narrowed the regression to extra custom-route
  pinning on ungated raw consumers, rather than business/control scoring.
- [mqtt-close-gated-contact-fix](mqtt-close-gated-contact-fix/results.json)
  passed three narrow original checks after preserving raw mapped bootstrap and
  applying new route pinning only to admission-gated native contacts. The full
  resumed MQTT group then passed. No debug instrumentation remains in source.

Intermediate standalone directories additionally retain actual preference,
UDP/relay, native-relay/TCP, disabled-origin, physical termination, unknown
binding and surviving-control results. The final fourteen-case matrix is authoritative rather
than combining those older runs into a fictional single run.

## Environment and cleanup

Linux IPv4 loopback, owned Mosquitto Docker image `eclipse-mosquitto:2`, real
native TCP framing/UDP sockets and an owned in-process native HTTP relay are
qualified. No other-platform, public-facility availability, unknown-member NAT
composition or native MQTT-first relay-only upgrade is claimed. Browser checks
are compilation only. Private fixture keys, secrets and SQLite remain temporary;
logs contain no exported session binding or admission material. No packet capture
or passing marker is described as a security audit.

The runner owns only its named brokers/anonymous volumes and cleans them in
`finally`; receipts retain actual cleanup exits. Tests close their endpoint,
node, proxy and native-relay resources. The independent [Standards](standards-review.md), [Spec](spec-review.md) and
[two-axis aggregate](code-review.md) preserve initial findings and final
supplements: Standards has no hard violations and two non-blocking duplication
judgements; the Spec P2 has a genuine RED/GREEN correction and no remaining
finding. They are stored beside this index; the OS-temp handoff points
to these artifacts rather than duplicating them.
