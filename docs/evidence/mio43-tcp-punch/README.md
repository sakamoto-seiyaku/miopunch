# MIOPUNCH-43 TCP punching evidence

The [implementation/support note](../../notes/2026-10-02-tcp-punch-validation.md)
records exact configuration, protected coordination, Linux capabilities and the
old-SYN conntrack limitation. Tests use the approved public formal-node,
Endpoint/Connection/stream, identity, resource, store and socket seams. Real
TLS/QUIC, signatures, SCRAM, SQLite and kernel TCP/NAT are retained.

## Real first check and prescribed priority failure

`corrected-first-red` temporarily ran the corrected public test entry against
the unimplemented b36ad3d baseline source, restoring only the implementation
agent's own edits afterward. The authenticated MQTT stage failed with
`UDP resource disabled`: no TCP data path could be released. `first-green`
then used the same durable-member fixture to complete TCP punching, actual
LogSync and business ACK on the original connection, with UDP/mapping/relay
disabled and two kernel NAT gateways.

`priority-cancel-red` exposed competing observations during simultaneous
attempts; `priority-socket-red` then reached coordinated TCP and actually
showed a SYN-SENT socket after endpoint cancellation. `priority-cancel-green`
records one side canceling after both started, both original sessions closing,
actual pending sockets0 and queue outstanding0. No later ticket was required.

## Subsequent single-condition checks

| Evidence directory | Observed behavior |
| --- | --- |
| `endpoint-dependent-green` | Actual destination-dependent port rewrites fail in about7s; no data release, no pending socket, repeated contact backs off |
| `udp-fallback-red` / `udp-fallback-green` | A permitted UDP alternative initially failed after TCP failure; same original session later completed actual UDP punching, sync and business |
| `loser-retirement-red` / `loser-retirement-green` | ss showed TCP retry recreating a loser even after UDP data release; original-connection retirement then made the actual loser socket table empty |
| `simultaneous-duplicate-real-red` / `simultaneous-duplicate-green` | Simultaneous attempts release one session; cached TCP contact initially bypassed dedup, then rejected duplicate while original stable ID remained live |
| `mixed-capability-red` / `mixed-capability-green` | TCP/UDP control formats initially crossed when one node disabled TCP; bilateral encrypted capability exchange selected allowed UDP with TCP probes0 |
| `asymmetric-observer-red` / `asymmetric-observer-fixed-green` | Independent Spec review exposed a real one-sided observer timeout crossing TCP/UDP frames; bilateral stage results then retained the original session for real UDP sync/business, without peer TCP sockets |
| `candidate-permission-green` | One node disables the exact remote TCP candidate; both observe successfully, agree to skip TCP and transfer over permitted UDP without creating a peer TCP socket |
| `wrong-target-green` | Real TLS rejects a wrong expected node key before TCP data; correct target subsequently transfers from the same durable nodes |
| `linux-interface-bind-green` | Listener, observer and actual dial reuse each private interface's fixed port; real dual-NAT TCP transfer succeeds |
| `cancel-retry-after-flow-expiry` | Cancel one attempt with endpoint alive, socket0; expire only owned peer conntrack entries, then new stable ID/same node identity and actual TCP business |

Each directory has exact command exits in `results.json`, real worker logs,
fixture source, packet capture and gateway rules/conntrack receipts. The runner
uses the executable reported by ordinary Cargo builds. Reusing a worker only
between unchanged source runs does not establish a new frozen-executable gate.
Raw stores/keys and live services are temporary, owned fixture resources.

## Diagnosed limits and fixture/compiler failures

The initial `first-red`, `first-behavior-red`, `first-green-attempt` and
`first-green-diagnostic` entries were compiler/missing-ALPN fixture errors;
they are not capability REDs. `corrected-first-green` exposed actual
SO_REUSEADDR-only AddrInUse. `reuseport-green-attempt` used a gateway that sent
RST for unsolicited SYN, outside the later declared supported silent-drop
condition. `silent-syn-green-attempt` reached TCP path validation but checked
asynchronous MQTT path removal too early; the corrected first-green waits for
public closure events before transferring real data. The first two
`simultaneous-duplicate-*red` build failures likewise are not behavioral REDs.

The `cancel-live-endpoint-red`, `cancel-live-endpoint-green`,
`cancel-retry-instrumented` and `cancel-retry-observer-linger` receipts preserve
bounded failure when immediately reusing an old TCP SYN conntrack tuple. The
last diagnostic pcap showed the gateway leaking a private-source SYN-ACK after
old sequence-state validation failed. The socket table proved the SDK's pending
socket was already gone. The observer-close hypothesis did not fix it and its
source change was reverted. Successful retry evidence expressly models expiry
of only owned peer flows; there is no claim that the client can reset foreign
NAT state or that arbitrary immediate tuple reuse succeeds.

`asymmetric-observer-green` is a compiler failure while implementing the review
fix (QUIC ReadExactError conversion and an incremental compiler diagnostic),
not a behavioral GREEN. The corrected `asymmetric-observer-fixed-green` is the
actual successful worker run. Both endpoints now keep the same encrypted TCP
coordination stream through bilateral observation, candidate permission and
validation results, so a one-sided failure selects the same fallback phase.

`udp-fallback-green-attempt` additionally diagnosed a bad QAD redirect: the
second WAN interface redirected to an unbound local address. The corrected
fixture binds the real observer to7842 on100.64.27.1. Packet capture includes UDP
for alternative-path runs. Temporary `[DEBUG-mio43]` instrumentation is removed
from runtime/test/runner source; its diagnostic raw logs remain as evidence.

## Final affected verification

`final-acceptance/results.json` records the final nine real topology/lifecycle
cells sequentially, first positive then the prescribed priority cancellation.
`final-short/results.json` records the affected MQTT bootstrap, resource policy,
formal native admission/proofs, MQTT purpose boundary, TCP direct/IPv6, framing,
all-targets, browser WASM and formatting checks. Both final receipts report
`passed: true`; each individual command returned exit0. Their final readback is
authoritative, rather than an inference from earlier source versions.
Already verified unchanged slow UDP NAT matrices were omitted under the user's
standing instruction. No unrelated archived Go/OpenSpec gate is applied.

The targeted review fix is checked separately in `review-fix-acceptance` (the
original nine real cells plus observer failure and candidate denial) and
`review-fix-short` (affected public regression/compile/format checks). Their
individual exits are all0 and both final `passed` flags are true, establishing
completion after the fix. The short logs report89 passing tests. Owned short
broker cleanup returned0 for both containers.

Independent [Standards](standards-review.md), [Spec](spec-review.md) and
[aggregate](code-review.md) reports record the initial P1, its real reproduction
and accepted fix. Final Spec has0 findings; Standards has0 hard violations and
one optional test-maintenance finding. Platform/NAT limits in the note remain
part of every completion claim.
