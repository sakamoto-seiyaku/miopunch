# MIOPUNCH-41 TCP gateway mapping evidence

Implementation and scope:
[TCP mapping note](../../notes/2026-10-02-tcp-portmap-validation.md).
The approved public seams are formal node startup/admission, resource candidates
and contacts, normal Connection streams, real sockets and gateway leases.
The lab uses actual NAT-PMP wire requests and Linux DNAT/FORWARD rules, original
iroh TLS/QUIC, signed authorization, SCRAM admission and private durable stores.
The remote namespace has no route to the node's private LAN. Gateway allocation
alone is never counted as successful node reachability.

## Accepted real scenarios

Each directory records actual command exits in `results.json`, worker/gateway
logs, real forwarding state and packet capture. Successful cases were introduced
one at a time after the first success and priority failure. Paths below identify
the accepted evidence, with historical failed attempts retained separately.

| Scenario | Actual evidence and behavior |
| --- | --- |
| First real TCP mapped formal communication | [resumed-positive](resumed-positive/results.json): public100.64.41.1:41041 reaches real listener; expected identity, manual approval, saved admission materials and member stream round trip; UDP data sockets absent, no relay |
| Priority TCP close/UDP ownership isolation | [resumed-close-isolation](resumed-close-isolation/results.json): actual same-IP/private41041/public41041 UDP socket and mapping still echo after TCP closes; foreign TCP/UDP49041 rules retained |
| Lease failure | [resumed-expired](resumed-expired/results.json): failed renewal withdraws candidate; independent new dial to old address fails; established authenticated connection still transfers; fresh permitted LAN contact succeeds |
| External port replacement | [changed-policy-green](changed-policy-green/results.json): external42041 replaces41041; old route fresh dial fails, new route fresh dial succeeds; same established session transfers |
| Gateway restart | [restart](restart/results.json): real gateway process PID changes, volatile TCP rules removed, renewal obtains42041; old route fresh failure and new route success are separate from the existing session |
| Successful renewal | [resumed-renewed](resumed-renewed/results.json): two successful renewals beyond the original six-second lease, retained candidate and fresh authenticated connection |
| Mapping Disabled | [resumed-disabled](resumed-disabled/results.json): no gateway request or protocol probe |
| Mapping Disabled/advanced factory | [resumed-disabled-advanced](resumed-disabled-advanced/results.json): explicit mapping-capable factory and raw builder opt-in cannot bypass the saved disable |
| Global TCP Disabled | [resumed-tcp-disabled](resumed-tcp-disabled/results.json): no TCP request; independently permitted real UDP mapping still allocated |
| Global TCP Disabled/advanced factory | [resumed-tcp-disabled-advanced](resumed-tcp-disabled-advanced/results.json): explicitly added TCP factory sends no gateway request under global TCP disable |
| Initial allocation rejection | [resumed-initial-failure](resumed-initial-failure/results.json): real TCP NAT-PMP rejection, no public TCP candidate; permitted LAN still completes formal admission and data |
| Allocation then address-query timeout | [resumed-allocation-timeout](resumed-allocation-timeout/results.json): acknowledged TCP lease is released before publication, foreign rules retained |
| Allocation then address-query cancellation | [resumed-allocation-cancel](resumed-allocation-cancel/results.json): endpoint close cancels pending query, bounded owned TCP cleanup |
| Renewal then address-query failure | [resumed-renewal-address-failure](resumed-renewal-address-failure/results.json): acknowledged renewal cleanup withdraws old candidate; new dial fails while existing authenticated stream still works |

Changed/restart passed before the explicit pause; production behavior is unchanged
since those successful runs. The resume changed a previously unexecuted advanced
worker's store selection, case-result wording and formatting. New normal-build
receipts identify the resumed cases. No deleted pre-pause binary is claimed to
exist or to be a post-cleanup build. The controlled six-second grant is a lab
lease, not a promised hardware-router timing or cross-operator SLA.

`disabled-protocol-packets.txt` is empty for accepted cases: PCP and UPnP were
not attempted. The priority UDP mapping uses a genuine opcode1 allocation and
an actual UDP socket; TCP allocation/release uses opcode2. Same numeric ports
are not interchangeable protocol leases. Final rule snapshots preserve the
foreign owners. Existing conntrack circuits may remain valid after rules change;
real streams on the retained Connection and new old/new-route contacts verify
those distinct outcomes, without silently recreating an old connection.

## Actual first RED and priority failure

[positive-red-runtime](positive-red-runtime/serve.log) reaches the real lab and
fails after8s because no TCP mapping candidate is published. The earlier
[positive-red/build.log](positive-red/build.log) is only a test API compilation
correction, not a real network RED.

[positive-green-compact-build](positive-green-compact-build/serve.log) and its
remote worker complete mapped formal admission and data, then the wrapper fails
on cleanup: the completed NAT-PMP mapping's release encodes UDP.
[close-isolation-red](close-isolation-red/lab.log) immediately shows actual UDP
echo failure after TCP deletes that independent same-port lease. After retaining
the allocated protocol in Mapping,
[close-isolation-green-clean](close-isolation-green-clean/results.json) passes;
the final normal-build resumed priority receipt also passes.

[expired-retry](expired-retry/unreachable.log) is a fixture error: a new worker
tried to bind the still-running old worker's fixed remote listener port. Remote
workers now bind ephemeral ports.
[changed](changed/unreachable.log) accidentally probes the replaced current
address instead of the saved old address; `changed-policy-green` corrects that
fixture and proves separate old-route failure/new-route success.

## Affected short regression and compilation

[affected/receipt.json](affected/receipt.json) records **53 meaningful ordinary
checks passed**: TCP carrier2, native probe11, resource policy26, formal direct
TCP11, UDP configuration2 and the bounded unavailable-strategy admitted-TCP
check1. One additional inert gateway worker entry is excluded from that count.
The [integration log](affected/integration-green.log) and individual logs give
actual test results; all completed commands returned0.

[affected/udp-release](affected/udp-release/results.json) is one genuinely
affected real UDP mapping/release regression. Workspace/all-targets typecheck,
WASM compilation and modified-source formatting pass. WASM is compilation only.
Initial host-socket attempts in `affected/carrier.log` and
`affected/integration-sandbox.log` failed with sandbox EPERM; the same commands
passed with actual socket access. No product fix was made for that environment
restriction.

Previously verified unchanged slow NAT algorithm/probability matrices were
excluded under the user's standing policy. This is not a full-workspace runtime
suite or a new NAT success-rate denominator.

## Storage pause and reproduction

The pre-pause `positive-green*` and early `close-isolation-green*` build attempts
include actual filesystem exhaustion and one command-line profile correction;
they are not runtime successes. Only this ticket's generated caches, unused
outputs and failed link temporaries were cleaned. A no-space formatter emptied
previously clean resources.rs; it was restored from baseline and only41's edit
reapplied before the successful builds.

The user explicitly paused implementation for project target-cache maintenance,
then explicitly resumed. Source/WIP/logs survived. The old target was removed;
`resumed-positive/build.log` records the new normal cold build using shared Cargo
line-table defaults, without special profile overrides. Historical stripped
receipts retain their actual older profile generation.

```sh
rtk proxy python3 -B scripts/check_tcp_portmap.py --case positive --output NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_portmap.py --case close-isolation --output NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_portmap.py --case expired --output NEW_DIRECTORY
```

Use `--case` for one behavior. `--worker` can reuse Cargo's current worker while
sources are unchanged; rebuild after edits. No executable freezing/source archive
or root audit gate is required. `--strip-debug` remains an optional constrained
Linux build override. Private keys and stores exist only in lab temporary paths;
owned workers, namespaces and container are removed on every result. Source
provenance deltas are documented in both vendored UPSTREAM.md files.

Independent [Standards](standards-review.md) and [Spec](spec-review.md) reports
and [aggregation](code-review.md) retain both axes: Standards0 hard violations
and2 optional fixture-maintainability suggestions; Spec0 findings. Runtime
source/tests/runners were unchanged after the reviewed snapshot. This ticket delivers controlled Linux IPv4 TCP NAT-PMP;
PCP/UPnP, hardware routers, other platforms, cross-operator behavior, browser
gateway access and TCP punching remain unclaimed.
