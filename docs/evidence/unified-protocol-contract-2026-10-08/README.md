# Unified protocol contract evidence

2026-10-08. The source contract and eight finite SDK cases are recorded here.
MIOPUNCH-82 and the complete MIOPUNCH-84 public matrix remain unfinished.
MIOPUNCH-85 is Done after its local acceptance checks, final source review,
scoped commit and complete Plane readbacks. Functional success and physical WAN qualification have separate results.

The migrated member, admission, precode and Replica entry points use the strict
common version1 protocol. Old browser member ALPNs, v2/v3 capability exchange,
fixed-role bootstrap callers, legacy fixture modes and the standalone old RTC
upgrade wire were removed. Shared private relay-candidate validation remains
inside the common coordinator. Native UDP/TCP mode algorithms and the iroh/noq
core were unchanged. Other explicitly registered purposes retain their existing
permissions. Seventeen separate command checks verified that retired fixture
entry points reject execution before output preparation, compilation or actors.

Current Native and test builds passed through the shared target with locked,
pinned dependency graphs. All 24 selected product checks passed: eleven common
protocol, four admission, three negotiated accept, three private relay and three
TCP cases. The relay group's initial sandbox socket-bind failure remains recorded;
the same pinned binary passed when allowed to use local sockets. Earlier failed
builds and observer failures remain preserved.

MIOPUNCH-85 fixes the real TCP contact-registry omission exposed by current SDK
integration. TCP capability was permitted and installed, but the restricted
contact registry discarded its address before I/O. Registering that existing
transport when permitted allows the TCP-only case to complete new purpose2
admission, save Complete, reopen, transfer and verify a nonempty purpose3 Replica,
and complete an explicitly registered business roundtrip. Disabled TCP is not
dialed; the wrong expected TLS identity cannot complete admission. The permanent
TCP helper passes its existing 12-second incoming deadline.

A later Native fixture preparation failure reproduced on both Host and Pixel.
Its old raw `Endpoint::connect` call omitted the caller context required by the
current protocol. The fixture now passes one explicit 12-second local preparation
deadline to `connect_with_opts` and incoming acceptance. The same real independent
member preparation passed on Host and actual Pixel. Its separate 45-second public
business caller is unchanged. This is the only additional source change after
the preceding current SDK cases.

The resulting Native and Android builds each rebuilt only that fixture. The other
five pinned Native executables are byte-identical to those used in the earlier
passing cases. Product-test and WASM selected input graphs remain current and
were reverified, so they were not rerun for this isolated fixture change. WASM's
seven full-snapshot changes outside its selected graph are declared; there is no
claim that its entire historical snapshot equals the current repository.

| Finite SDK case | Actual result and scope |
| --- | --- |
| Browser→Native | Actual Pixel Browser and Host Native: normal public EMQX TLS/WSS, restricted manual admission, wrong TLS claim and unapproved data rejection, interrupted material completion, both identities reopened, nonempty Replica verified/saved/applied and persisted, original-M RTC PING/PONG, member-removal rejection. |
| Native→Browser | Actual Host Native initiates and Pixel Browser answers. Current member gates, new wire, initial and new final fresh ACK, expected peers, both original-M stream directions and saved identities pass within the explicit 12-second callers. |
| Browser↔Browser | Independent Host Chromium and Pixel Chrome both take initiator and responder roles. Persisted peer grants, current gates, fresh initial/final ACK and bidirectional original-M streams pass. |
| Relay-first improvement | Controlled Host pair starts business through its owned local relay, then explicit `PreferDirect` commits and selects RTC on the same M/binding/stream. Public EMQX carries signaling only. Default `Quality` may retain the faster relay; its earlier overstrict selection observer remains failed. |
| RTC no path | A real RTC peer is created and retired. The current SDK returns `DeadlineExceeded` at 60,004.465 ms for its 60,000 ms caller, within the existing observation allowance. The healthy owned relay M, binding, stream and PONG survive. This is a controlled path failure, not a WAN outage claim. |
| Actual carrier cut and repair | Normal close of the exact owned live Browser RTC PeerConnection removes the sole RTC carrier after the old relay path was closed. Public SDK repair returns in 951 ms on the same M/binding/stream; replacement-PC and fresh ACK proofs pass, with later business delivered after the existing repair cap. No member close, permission change or token invalidation is used as the fault. |
| RTC-disabled relay-only | Actual Host Browser with `rtc_enabled=false` and Host Native use an owned local relay. Member gates, new protocol, fresh ACK and sequence0/1 on the same M/stream plus PING/PONG pass. No RTC candidate, PC, DataChannel, association or pump is created. Browser advertises no RTC role; Native may still advertise its own supported Answerer capability. This does not qualify the failed public-relay case. |
| Actual entry fault | Actual Host Native/Pixel Browser: normal close of the owned page's exact broker WSS entry leaves its existing RTC business working. After explicit closure of that old M, a new 12,000 ms attempt reports `member connection caller deadline exceeded` at 12,064.49 ms, within the existing 500 ms local observer allowance. No ordinary handle remains; saved identity/materials are unchanged. No global network setting changes. |

Successful cases passed their independent owned cleanup and source/artifact
postconditions. SDK connections, PC/DC owners, pumps, queues, listeners, processes
and forwarding owners retire; immediate and late observations remain in the
receipts. Raw captures, temporary identities, private addresses, SDP, credentials,
exporters and transcripts stay private. Capturing other traffic on an interface
is not a claim that all captured traffic belongs to the SDK.

Native→Native public business and its reverse remain unqualified. The actual
Android shell run reached preparation but normal platform TLS panicked because
an Android application context was not initialized. Separate Host Native attempts
had no ready public MQTT contact, including an explicit-facility attempt with TLS
handshake EOF. The first failed case was inspected before expansion; no reverse
was promoted or fabricated. These failed runs have verified owned-process and
capture cleanup, but no successful graceful SDK/socket-release qualification.

Public RTC-disabled relay-only also remains failed: its first run completed
admission, reopen, nonempty Replica and a relay member gate/fresh ACK, then an
external observer requested an RTC-only stable-ID field before business. With
that observer corrected, the next public run returned `connection lost` during
admission. The separately passing owned-relay case establishes the functional
semantics and does not turn either public failure into success. IRC's earlier
full-SDK qualification gap remains; its branch was not expanded. TURN was not
configured.

Both physical-data and physical-WAN qualification remain false. The Host kernel
route and owned socket were observed, but its Windows upstream route remains
unknown. Pixel's selected route was observed on its actual interface, while its
default Wi-Fi agent was not marked validated and therefore did not meet the full
non-overlay predicate. The latest complete-story two-end packet correlation is
false; a historical different-source success is not substituted for it. A real
different-network, non-overlay site with observed selected data routes is still
needed; device movement and system network changes are left to its operator.

The user's explicit authorization of any related tests superseded the earlier
approval blocker. Rejected commands did not start; later accepted executions and
all original failures are preserved separately. Source fixes, field-observer
corrections and public-service failures have distinct receipts. There is no
complete public-matrix or NAT-success claim.

See [results.json](results.json) for whitelisted source/artifact/command/cleanup
hashes, actual caller measurements, preserved failure boundaries and qualification
limits. Twenty-two source paths belong to this change; 234 unrelated pre-existing
WIP files remain unchanged. [Standards review](standards-review.md) has zero hard findings and one optional
cleanup suggestion. [Spec review](spec-review.md) has zero required source fixes
and identifies the two disclosed environment/public-matrix acceptance gaps. Both
reviewers were verified as GPT-6.1-sol/high/default; no source changed after review.
The optional obsolete `Option<PreparedPath>` result state is parked. Source commit is `07d3a62776928a5799747b4dd1b9cb7935931c5c`, without push.
M85 is Done under its actual parent M84; M82 is In Progress with
`ready-for-human`, and M84/parent69 remain In Progress. Original M70–84 has
13 of 15 Done, with M82/M84 unfinished. The additional M85 fix does not turn
the original fifteen into a complete matrix. Plane readbacks preserved prior
comments, full protected task fields and native relationships.
