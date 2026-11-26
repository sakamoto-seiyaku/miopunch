# MioCube native dual-stack TCP

MIOPUNCH-198 implements ordinary TCP listening and candidate announcements under
MIOPUNCH-195. The baseline is `05b24362`. The first implementation commit is
`d1d507ae`.

The native default binds separate IPv4 and IPv6 listeners under one TCP carrier.
An unavailable family leaves the available listener usable. Explicit listener
configuration and TCP disable remain effective. The carrier advertises concrete
interface addresses and gateway mappings, and watches subsequent changes.
`Endpoint::network_change()` also refreshes its owned interface monitor.

The admitted member session sends every permitted, deduplicated ordinary TCP
candidate using the established two-address message shape and byte boundary.
Messages continue under QUIC stream-credit backpressure. Candidate validation
keeps its existing two concurrent slots and original-session ACK qualification.
Subsequent messages and address updates use the same receiver.

Local bootstrap material uses a common representative selector at its existing
capacity. Invitations and guest provider hints retain 32 addresses, and local
maintenance contact submissions retain eight. Relay/contact and both direct
families receive opportunities. Authenticated receive checks and stored contact
limits retain their existing checks. Full ordinary TCP exchange uses the carrier
watcher independently of these bootstrap hints.

## Actual entry results

Linux user/network namespaces ran two admitted `NativeClient` instances. UDP was
disabled throughout the TCP acceptance cases.

| Behavior | Evidence |
| --- | --- |
| Default IPv4 and IPv6 TCP first contact, bidirectional PONG | `/tmp/miopunch-198-first-ns.log` |
| Only reachable TCP candidate is third or seventh, and a new candidate is added later | `/tmp/miopunch-198-notified-update.log` |
| All three relay improvement cases keep the original stable connection and exchange PONG in both directions | Same log, `TCP_LATER_DATA` records |
| Forty IPv4 aliases produce 43 public routes, ordinary invitation and guest provider material encode with 32 hints | `/tmp/miopunch-198-guest-boundary.log` |
| Stream-credit blocking survives beyond the former six-second whole-set timeout | `/tmp/miopunch-198-backpressure-final.log`, five `STREAMS_BLOCKED_UNI`, candidate 17 ACK-ready at 48.224 seconds, same original stream |
| IPv6 disabled, default IPv4 carrier works and caller QUIC idle/congestion settings hold | `/tmp/miopunch-198-v4-only.log` |
| Explicit TCP disable preserves relay business | The passing existing `disabled_tcp_keeps_the_same_relay_stream` case in `/tmp/miopunch-198-regression/resources.log` |

The backpressure regression uses an authenticated identity-specific member policy
at the existing TCP carrier seam. NativeClient cases independently cover product
membership, service authorization and selected TCP paths. The lower regression
keeps a healthy UDP original and verifies the late TCP path's fresh ACK and
continued original stream without imposing a new path-selection preference.

## Regression and review

One complete selected regression ran fixed library, `tcp_direct`, `resources`
and `network_maintenance` targets, with two owned MQTT brokers and the built CLI.
The initial results are retained in `/tmp/miopunch-198-regression`.

- Library had 67 passes and one pre-existing active-budget failure already
  reproduced on the pre-196 baseline.
- TCP direct had ten passes. Its allowlist and received-revocation failures
  reproduced on `05b24362` with the same operations and assertions. See
  `/tmp/miopunch-198-baseline/runtime/results.json`.
- Resources initially had 49 passes. Four TCP fixture failures came from an
  additional normal listener becoming reachable. The shared fixture now excludes
  those extra listeners from both peers. The explicit-contact fixture uses an
  empty existing announcement override so its manual contact remains its trigger.
  All four original assertions passed in `/tmp/miopunch-198-tcp-fixture-runtime`
  and `/tmp/miopunch-198-wildcard-final.log`. Four remaining failures are the
  already characterized bounded-backoff, interrupted-observation, enhanced-UDP
  timing and high-level p2panda mDNS cases recorded by the preceding tickets.
- Seven maintenance cases rejected the expanded local contact list before
  entering their behavior. All passed after using the existing eight-hint
  boundary. See `/tmp/miopunch-198-hint-runtime/results.json`.
- The enhanced-only TCP fixture now suppresses ordinary direct advertisements
  with an endpoint-local `miopunch-test-hooks` Builder option. It keeps its empty
  offer and traversal capability, actual listener, STUN and original assertions.
  Commit, retirement/requalification, standby retry and cancellation passed in
  `/tmp/miopunch-198-enhanced-fixture-runtime/results.json`.

Three maintenance limits remain recorded. The stale-contact and network-change
observation cases have preceding baseline failures. The same-generation TCP
cooldown case completes original-stream data but fails its final empty observer
queue assertion on the current artifact. A retained 197 artifact passed the same
case. The extra observations occur before the reported two-sided selected-path
ACKs and business qualification. Its scheduling logic is unchanged in this ticket.
The main thread directed preserving the assertion and handing this exact comparison
to MIOPUNCH-203. Reproduce with
`.artifacts/miopunch-198-enhanced-fixture.maintenance --exact active_original_application_members_retry_tcp_after_same_generation_cooldown --nocapture`.
The comparison logs are `/tmp/miopunch-198-cooldown-focused.log` and
`/tmp/miopunch-198-cooldown-197-baseline.log`.

Independent Standards review found no violations. Spec review found the whole-set
timeout and verified its repair. Focused rechecks covered the notification hook,
local hint consumers and fixture changes, with no remaining findings.
Native library check and the shared-source WASM library check passed in
`/tmp/miopunch-198-native-final-check` and `/tmp/miopunch-198-wasm-final-check`.

## Platform qualification

Runtime evidence covers Linux. Windows, Android, macOS and iOS use the same default
listener construction, family selection and ordinary announcement code. Their
native runtimes were not exercised here. Android's netwatch backend has no route
events, so its host must deliver the existing explicit endpoint network-change
notification for later interface refresh. TCP port reuse and coordinated mutual
dialing keep their existing platform constraints and are covered by MIOPUNCH-203.
The TCP address format continues to represent IPv4 and unscoped IPv6.
