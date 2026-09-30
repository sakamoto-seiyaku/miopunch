# MIOPUNCH-58 evidence

Acceptance and limitations are described in
[the validation note](../../notes/2026-09-30-nat4-pair-validation.md).
Only the final `matrix/` contributes to the bilateral success-rate claim.
`tuning/`, `failures/` and `boundaries/` answer separate checks. Never pool
exploratory rows with the final matrix or treat a harness error as a failed
application path.

The controlled kernel gateways have explicit per-destination SNAT rules,
including different ports on the same peer IP, and APDF reverse-tuple filtering.
Their preflight conntrack and accepted/dropped packet counters are saved in each
trial and cleared before the application starts. This demonstrates the
algorithm's conditions under programmed allocations, not the frequency of those
conditions in household or mobile NATs.

Each final trial saves endpoint logs, WAN SLL2 pcap, gateway rules/counters,
conntrack, preflight evidence and runner status. Evidence contains disposable
public node identities, candidate addresses and random coordination rounds;
private keys and admission secrets remain in temporary fixture directories and
are deleted by the harness.

The runtime snapshot is frozen while an experiment executes. Live edits to a
Bash file already being interpreted can corrupt reads; the final experiment
mounts a separate read-only copy of scripts and binaries. Capture checks use
SLL2 packet type 0 (WAN ingress from a gateway) to count each peer send once.
They do not merge timestamps, and verify exact prime payload and predicted UDP
source/destination ports. QAD replies originate on the WAN and use packet type 4
for outbound response cost. `--immediate-mode` avoids losing the capture's final
kernel-buffered packets when tcpdump exits.

| Exploratory directory | Why excluded from final acceptance |
| --- | --- |
| `initial/` | Initial interrupted trial before the strict mapping fixture. |
| `peer-ip-only-abandoned/` | 23-row exploration using a peer-IP rule and MASQUERADE escape; adjacent peer destination ports were not independently proven APDM. |
| `strict-apdm-preflight/` | A standalone strict APDM/APDF preflight and application smoke check, not the complete paired matrix. |
| `four-combination-calibration/` | Four representative combinations, 32 trials; baseline 0/16 and auto 16/16 at the earlier 2+1 application-prime budget. |
| `pre-topology-validation/` | A full-matrix attempt stopped at row 17 because the harness whitelist omitted +1×+2; the application never started for that row. |
| `pre-diagnostic-matrix/` | A 1+1 attempt stopped at row 16 when a live shell script edit corrupted a read. Its logs predate actual control-frame counting and its capture can omit tail packets. |
| `pre-fault-injection-order/` | Seven completed checks followed by a preflight failure: a peer mapping jump was injected before the APDF classifier, so this was a harness error. |

The 32 tuning trials compare first-side one versus two application primes with
both second-side arms fixed at one. They predate the diagnostic fixes: the
`CONTROL_MSGS=4` field describes planned exchanges before path promotion, not
five actual coordination frames including promotion/commit. Their application
successes and path delays inform the budget choice, but final cost/timing claims
use the reviewed final matrix captures and real frame-event logs.

The final suite log records all workspace tests with a dedicated real MQTT
broker. Three pre-existing ignored tests remain excluded: one flaky upstream
gossip test and two upstream authorization documentation examples. The broker
and experiment containers are temporary; unrelated containers are untouched.
