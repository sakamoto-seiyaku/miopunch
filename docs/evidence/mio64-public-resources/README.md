# MIOPUNCH-64 public Linux / Android evidence

Specification: [Linux 与 Android 经公网 MQTT、STUN 完成真实连接与恢复验证](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d07357ae-03a0-499f-b59a-891f41adc713).
Implementation choices, public-source references and the reproducible command
are in the [integration note](../../notes/2026-10-03-public-resource-integration.md).

The physical run uses the shared native binaries, the foreground JNI Android
shell, actual EMQX MQTT, actual Cloudflare STUN and the official number0 HTTPS
relay. Local regression facilities are separate. An owned loopback CONNECT
forwarder passes unchanged TLS bytes to a recorded actual public relay peer;
it implements no relay or STUN protocol and supplies no fake result.

## Qualified results

| Check | Actual scope |
| --- | --- |
| Default resources | Public relay admission, one precode redemption, persisted authorization and business PONG; unchanged built-in resource table |
| STUN/TCP | Both platforms receive transaction-validated Binding replies from Cloudflare; query uses and retains the actual TCP carrier listening port; IPv4 and IPv6 mapped responses supported |
| STUN/UDP | Both actual Endpoint sockets send three kernel-accepted requests and time out; no public UDP response or reachability pass claimed |
| Missing UDP transport | Explicitly distinguished as a local prerequisite error; no imaginary socket send or external outage claim |
| FIRST | Protected public MQTT bootstrap upgrades the original Connection to the approved HTTPS public relay; real LogSync saves one new record, 2 → 3; actual business PONG; MQTT counters unchanged during business |
| PRIORITY | One owned relay-tunnel blackhole; bounded business failure and terminal old connection; fresh MQTT control attempt returns no business handle; restore/restart retains identity, three records and resource configuration, uses remain 1 |
| Disabled resources | Observation, Signaling and DataRelay remain disabled across restart; no observation, no MQTT advertisement, public data contact fails; explicit restoration succeeds |
| Physical topology | Pixel 6a / Android 16 API 36 / aarch64 and WSL2 Linux on the same private /24; UDP and LAN TCP data excluded, actual public relay required; independent-network direct traversal not available |
| Cleanup | Owned device directory and remote processes removed; tunnel sockets closed; device network settings unchanged |

## Receipts

- `results.json`: physical first/failure/recovery results, selected resources,
  TLS conditions, public relay peers and actual forwarded/dropped byte counts.
- `processes.log`: shared-node commands and diagnostics, including original
  connection IDs, saved record counts, bound socket addresses and MQTT packet
  counts. It contains public fixture identities and no private stores or
  invitation contents.
- `linux-dns.json`, `environment.json`: DNS answers and sanitized topology.
  Successful TCP observations include each actual destination and local port.
- `cleanup.json`: owned-device cleanup and absence of network-setting changes.
- `cleanup-fixtures.json`: preflight files, private OS-temporary fixture stores
  and the separate controlled regression broker removed after validation.
- `review.md`: independent Standards and Spec review, kept as separate axes.

## Regression and build checks

`checks/final-affected-suites-v3.log` records **105 passed / 0 failed**:
library 25, admission gate 18, MQTT bootstrap 8, native network 5, public
resources 6, resources 26, TCP direct 11 and UDP observation 6. The MQTT test
uses an actually running separately owned controlled broker via
`MIOPUNCH_MQTT_BROKER=127.0.0.1:18864`; the public acceptance uses EMQX instead.
The unchanged slow NAT matrix is skipped under the user's standing policy.

The Spec review found a TCP-observation supersession race after these checks.
`failures/tcp-supersession-red.log` reproduces an older pending query publishing
after a newer same-origin attempt. The per-origin cancellation fix rejects the
older query and prevents stale publication. `checks/tcp-supersession-green-affected.log`
records **33 passed / 0 failed** across the seven public-resource checks and
26 resource checks after that correction. The prior physical acceptance uses
sequential observations, so its public network results remain qualified.
`checks/post-review-native-check.log`, `checks/post-review-fmt.log` and
`checks/android-post-review-build.log` record compilation of the correction.

`checks/final-stun-diagnostics.log` additionally checks actual UDP binding
diagnostics and the four STUN boundaries after the logging correction.
`checks/final-native-check-v3.log` records the final all-target native check;
`checks/final-fmt-check-v3.log` is empty because formatting passes.
`checks/linux-final-public-build.log`, `checks/android-final-build.log` and
`checks/android-build.json` document actual Linux/Android builds and the
Android toolchain/JNI shell. Existing unrelated unused-mut warnings remain.

## Original failures

`failures/android-default*.log` retains the original standalone Android CLI
panic caused by uninitialized JNI context in DNS. No certificate bypass is
used to repair it. `failures/mqtt-connack.json` records Mosquitto EOF and HiveMQ
timeout; `mqtt-alternate-connack.json` records the actual EMQX CONNACK and the
separate verified-TLS Mosquitto probe. `initial-public-results.json` retains
the first node run's external and IPv6 STUN parser failures. An independent
temporary-socket wire probe is excluded from node mapping evidence.

`public-snapshot-failure*` records the public run rejected for a closed MQTT
path still appearing in its snapshot. `closed-path-snapshot-red.log` reproduces
the same API bug deterministically; `checks/closed-path-snapshot-green.log`
passes after the Noq closed-status guard and snapshot filter. The existing
packet gate had already forbidden signaling packets before release.

`public-validation-failure*` retains the next bounded connection failure
before a business handle was returned. The original transparent proxy polled
both directions with serial 200 ms receive waits, adding artificial delay to
relay handshakes; it now waits on socket readiness. The failure itself does not
establish a general external-service outage or a membership failure. Final
validation errors are logged when observed, before the peer-result exchange.

Other `*-red.log` receipts show the missing public STUN seam, protocol/QAD
separation, TCP binding seam, IPv6 mapping and missing-UDP prerequisite checks
before implementation. Compiler errors establish missing APIs; successful
wire integration and runtime regressions are recorded separately.

Private fixture databases, node keys, invitation JSON, SSIDs/MACs, device
serials and API credentials are deliberately excluded. OS-temporary failed
run directories contain no evidence used as a second task tracker.
