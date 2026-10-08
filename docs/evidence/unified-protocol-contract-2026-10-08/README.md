# Unified protocol contract evidence

2026-10-08. The source contract, eight earlier finite SDK cases and six
restored OP13/Pixel SDK executions are recorded here. MIOPUNCH-82 environment
qualification passed collector review and is Done in Plane; the complete MIOPUNCH-84
public matrix remains unfinished.
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

The restored OP13 cellular/Pixel Wi-Fi site now passes five additional current
SDK executions, with Host providing only assets, wired control and evidence:

| Restored-site case | Actual result |
| --- | --- |
| Browser→Native whole story | 69,782.113 ms. Normal public TLS/WSS, admission/reopen, four verified/saved/applied Replica records with persistence, original-M RTC PING/PONG and removal rejection. |
| Native→Browser | 44,871.192 ms. Both original-M stream directions, initial/final fresh ACK, saved identities and both 12-second callers pass. |
| Public RTC-disabled relay-only whole story | 166,848.693 ms across separate SDK operations. Normal public contact and selected public relay, admission/reopen, four-record Replica and original-M PING/PONG pass. RTC candidate/PC/DC creation remains zero. Individual SDK deadlines remain unchanged. |
| Native OP13→Pixel | 49,893.634 ms including preparation and cleanup; actual original-M native UDP business returns within the 45-second callers. |
| Native Pixel→OP13 | 45,241.182 ms including preparation and cleanup; reverse original-M native UDP business returns within the 45-second callers. |

Both Native directions retain current gates, expected identities, new protocol,
fresh ACK and original binding. Natural SDK retirement, immediate/late socket
release and verified durable material/record reopen pass. Their historical
control labels `host`/`pixel` identify OP13/Pixel actors in these private runners;
no Host SDK participant is used. Their own physical-WAN flags remain false:
these cases do not have selected-tuple two-end packet/route qualification.

Three failed Native-pair observer runs remain preserved. Android `ip -j` emitted
text; the first observer incorrectly parsed JSON. Next, only the broker URL had
been saved, so the existing explicit MQTT-facility configuration was applied.
Finally, the bounded stdout reader stopped after its in-memory retention limit,
blocking the SDK's pipe; OP13 required forced cleanup while Pixel returned its
actual 45,013 ms deadline. The corrected private reader drains stdout to bounded
private files and keeps memory bounded. It then passed both original directions.
No Rust SDK, transport algorithm or caller budget changed.

The first restored RTC story has physical route corroboration: opposite
CELLULAR/WIFI defaults, zero current VPN agents, selected nonrelay UDP, exact
routes to the actual default interfaces, live Native socket/inode/mark ownership,
and matching two-end selected packet payloads. Full captures match 57 distinct
Native→Browser and 61 Browser→Native encrypted DTLS application datagrams.
Encrypted captures do not identify an individual inner STREAM by themselves;
actual SDK business and unchanged original-M diagnostics supply that evidence.

The original collector conflated Android Internet `VALIDATED` with physical
non-VPN access. Its offline regressions reproduced both rejection of unvalidated
physical Wi-Fi and acceptance of concurrent VPN with validated Wi-Fi. The fix
retains validation independently and requires an observed default agent,
`NOT_VPN`, exactly one physical transport and zero current VPN agents. All 25
route checks pass. The selected tuple, route, live ownership/mark, distinct global
wire addresses and two-end packet gates remain required.

The fixed collector replayed every identical saved live read, with each operation
bound to its original command, and revalidated the original selected-pair packet
witness. This reevaluation qualifies physical data and WAN for that single RTC
story. Original raw receipts, their false flags and captures are unchanged;
Pixel `defaultValidated=false` remains false. This is offline evidence replay,
not another SDK execution. Browser socket UID/mark/protect remain unknown.
[returned-op13-site.json](returned-op13-site.json) records the old and new hashes,
actual runtimes, cleanup, failure boundaries and qualification scope.

Earlier Android platform-TLS, Host contact and public-relay failures remain
historical failures. The restored public-relay story qualifies its own selected
relay behavior, without relabelling earlier runs. IRC's existing full-SDK gap
remains; that failed branch was not expanded. TURN was not configured. A single
corroborated scene does not qualify every matrix direction or establish a NAT
class, success rate or universal no-overlay guarantee.

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
reviewers were verified as GPT-6.1-sol/high/default; the reviewed Rust source is
unchanged. The later Python collector fix receives a separate scoped review.
The optional obsolete `Option<PreparedPath>` result state is parked. Source commit is `07d3a62776928a5799747b4dd1b9cb7935931c5c`, without push.
M85 is Done under its actual parent M84. M82 is Done after
successful independent collector review and full Plane readbacks. M84/parent69
remain In Progress. Original M70–84 has
14 of 15 Done at the current tracker readback, with M84 unfinished. The additional M85 fix does not turn
the original fifteen into a complete matrix. Plane readbacks preserved prior
comments, full protected task fields and native relationships.

The separate [collector Standards review](op13-collector-standards-review.md)
has zero hard or heuristic findings. The [collector Spec review](op13-collector-spec-review.md)
has zero required source fixes and supports M82 closure for the observed scene;
it retains two M84 gaps. Both reviewers were actually verified as
GPT-6.1-sol/high/default. Collector and test hashes remain unchanged after review.

An earlier two-phone Browser pair stopped before any browser SDK call:
OP13 has no installed Chrome package or resolvable activity. Its actual exit1
and first page-launch failure remain. The provisioner naturally closed, owned
control listeners/reverse forwarding and captures retired. Because neither
Browser SDK started, the failed observer's missing RTC-allocation cleanup witness
is preserved and no Browser SDK graceful-close qualification is claimed.
That first attempt installed no application and changed no system network setting.

A subsequent authorized ADB install adds Titanium (the renamed Android Helium
project, package `io.github.jqssun.helium`) version 154.0.8037.126 on OP13. The
publisher APK digest, installed APK and signature verification match. The first
local install wrapper reported failure although the device completed installation;
that limitation remains recorded. No default browser or permission-grant flag changed.

The two independent phone Browser SDKs now both complete manual admission, save
both verified peer grants, reopen with the same identity/materials, and take both
initiator/responder roles. Titanium initiates in 7,946.400 ms; Pixel initiates in
7,417.450 ms, within the original 12,000 ms callers. Both original-M RTC stream
directions, current gates, new wire, initial/new final fresh ACK, post-story
persistence, natural SDK retirement and independent capture/control cleanup pass.
The whole finite story takes 86,674.671 ms across separate operations.

Five preceding failed SDK attempts remain preserved. Read-only owned storage
comparison rules out different network/secret/verified records. CPU sampling
shows the unoptimized Titanium WASM execution consuming most of its member caller;
its precise inner interpreter operation was not resolved. A same-source release
WASM build advances through member proof and offer/answer, without changing the
proof algorithm, dependency pins, committed profiles, browser JIT or caller budgets.
With Titanium's original `Disable non-proxied UDP` policy it still generates zero
ICE candidates and fails, while Pixel generates three. Titanium's own checked UI
confirms that policy.

Selecting `Default public interface only` temporarily in this newly installed
test browser then passes both directions with the same release SDK. The original
policy was restored and its checked selection verified after cleanup. This is
a declared browser configuration prerequisite. SDK admission and membership
checks continue to apply.
No OS network/VPN settings or JIT exceptions changed. The [Titanium documentation](https://github.com/jqssun/android-titanium-browser#webrtc-ip-policy) describes its
default IP protection and the public-interface option. The new release build used
the existing shared cache, with unchanged source/dependency locks and retained
3159 input, 198 source-copy and 264 depfile proofs.

[titanium-pixel-browser-pair.json](titanium-pixel-browser-pair.json) records the
whitelisted runtimes, artifact hashes, failed attempts, policy restoration and
cleanup. Its own physical-WAN flag remains false because selected-tuple route,
socket and two-end packet qualification was not collected for this case. The
earlier Native/Browser physical proof is not inherited. M84/parent69 remain
In Progress; IRC and the full per-case physical matrix are unfinished.
