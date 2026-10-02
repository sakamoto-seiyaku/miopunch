# MIOPUNCH-42 evidence

Rerunnable commands are in each `results.json`; logs and controlled fixture
scripts retain the actual observations. Interpretation and integration choices
are in [the validation note](../../notes/2026-10-02-mqtt-bootstrap-validation.md).

| Directory | Actual outcome |
| --- | --- |
| `first-red` | Test-shell compile failure; not behavioral RED. |
| `first-runtime-red` | Preparation fixture deadlock; not capability evidence. |
| `first-native-red` | Real initial RED: persisted admission succeeded but formal startup had no MQTT route. |
| `first-green` | Real MQTT/TLS reached the old Bootstrap-only NAT gate and failed; not GREEN. |
| `first-green-member-scope` | Original connection, same primary UDP socket, real LogSync saved=2 and business through controlled dual NAT. |
| `priority-cut-red` | Metrics feature compile failure; not behavioral RED. |
| `priority-cut-runtime-red`, `priority-cut-green` | Real stale-route RED, then withdrawal GREEN; original attempt closed, no data handle, 100 early retries add no attempt. |
| `priority-duplicate` | Independent subscriber replayed 97 packets; each side retains one TLS/data connection with real sync/business. |
| `other-broker-red`, `other-broker-green` | Real single-service restriction RED; failed broker plus another subscribed approved service GREEN. |
| `startup-udp-policy-red`, `startup-udp-policy-green` | Saved UDP disable on an already open node was stale, then rejected before traversal on next bind. |
| `simultaneous` | Both sides dial: two real TLS connections per endpoint, bounded independent handles and bidirectional business. |
| `cancellation` | Cancel during admitted restricted data stage; original resources close, later fresh identity succeeds. |
| `data-cut` | Formal tagged route, original UDP connection/sync/business, kernel DROP, bounded original failure with no MQTT business fallback or redial. |
| `control-only` | Reachable MQTT but peer UDP blocked: no business result, one opened/closed attempt, bounded early retries. |
| `wrong-target` | Real TLS expected-identity mismatch rejected. |
| `formal-short` | Seven meaningful real checks passed (reported eight includes inert lab-worker entry). |
| `affected-suite` | Interrupted at user's handoff request. Only MQTT tests and 26 resource tests completed; incomplete group is not a pass. Containers were manually removed. |
| `affected-suite-resumed` | PASS: unfinished native/gates/proofs onward, all 13 check groups exit 0 and two owned brokers removed. |

Earlier stages retain the then-current route format; later `data-cut`,
`control-only` and `formal-short` exercise the configured 32-byte origin-tagged
route. Earlier successes do not claim a final whole-suite result.

To reproduce the final controlled two-NAT scenarios using the current build:

```sh
rtk proxy python -B scripts/check_mqtt_bootstrap.py --case positive --output NEW_DIRECTORY
rtk proxy python -B scripts/check_mqtt_bootstrap.py --case control_cut --output NEW_DIRECTORY
rtk proxy python -B scripts/check_mqtt_bootstrap.py --case duplicate --output NEW_DIRECTORY
rtk proxy python -B scripts/check_mqtt_bootstrap.py --case data_cut --output NEW_DIRECTORY
rtk proxy python -B scripts/check_mqtt_bootstrap.py --case blocked --output NEW_DIRECTORY
```

The Docker image `miopunch-nat61-lab:bookworm` and an isolated privileged
container supply Linux network namespaces, EIM/ADF rules, captures, local
Mosquitto and standalone TLS QAD. No external broker or relay is involved.
The runner reuses the already validated MIOPUNCH-27 kernel topology, not its
entire slow NAT matrix. For affected short checks with owned real brokers:

```sh
rtk proxy python -B scripts/check_mqtt_bootstrap_short.py --suite --output NEW_DIRECTORY
rtk proxy python -B scripts/check_mqtt_bootstrap_short.py --suite --start-at native-gates-proofs --output NEW_DIRECTORY
```

The resumed checks include native admission/proof, carrier purpose, manual
admission, replica persistence, TCP direct, selected formal NAT budget/epoch
checks, library tests, all-targets typechecking, browser WASM compilation and
formatting. Already passed unchanged resource checks and slow NAT matrices
were not repeated, following the user's explicit test policy. WASM is a
compilation result, not native MQTT traversal in a browser.

The completed initial checks report 8 MQTT entries and 26 resource tests; the
resumed tests report 103 entries. Combined this is 137 passing entries,
including three inert subprocess-worker entries (one MQTT and two manual
admission), or 134 meaningful tests. All-targets with the existing NAT/replica
test features, browser WASM, root formatting, changed-fork formatting and Python
runner syntax checks also pass. The interrupted group has no final result and
is not counted twice. No product/runtime source changed after these checks.
