# Browser RTC: merged admission and controlled cross-network qualification

2026-10-04. This combines accepted STEP10 `a48dbe0` and isolated STEP11
`fc87779f`, whose parent was STEP9 `10831294`. It records fresh qualification
of the merged source. The earlier [first-material](2026-10-04-browser-wss-admission-materials.md)
and [isolated cross-network](2026-10-04-browser-rtc-cross-network.md) evidence
remains historical and unchanged. Formal acceptance and Plane status belong to
the root workflow; this note does not make a product adoption or default-service
decision. Canonical scope remains [STEP9–11 and shared boundaries](2026-10-03-browser-webrtc-replan.md).

## Semantic merge

The SDK retains genuine empty-applicant restricted A, exact applicant/request
approval, original Delivery caller clock, signed authorization/history checks,
strict IndexedDB CAS/oncomplete, material receipt and durable Complete before a
distinct new M. STEP11 adds the optional persisted `rtc_ice` startup profile;
absent settings retain `None` and skip serialization. Existing saved profiles
and settings reopen through the existing store checks. A profile grants no
membership or data permission and cannot replace each M's current proof,
initial and newer final-release ACKs, or exact route seal.

The native qualification fixture retains Create-or-Open admission mode,
targeted approval, private issued-code handling and original DATA_DIR reopen.
Its old forms remain valid:

```text
browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM
browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM admission
browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM legacy-no-rtc
browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM legacy-relay RELAY_URL
browser_wss_member_native DATA_DIR FACILITY_JSON TEST_CA_PEM rtc-relay RELAY_URL
```

Each accepts an optional trailing `--ice-profile FILE`. That path never becomes
the relay URL or a legacy-mode selector. Two fixture CLI tests cover old forms,
combined options and ambiguous/incomplete rejection. The existing profile
alias check also checks saved Observation URL disables before the fixture
imports its explicit observer resources. Opaque/authority/default-port/path
spellings identify the same permitted or disabled STUN observer. Global
resource-origin rules and defaults are unchanged. The immutable bounded
profile accepts at most two numeric IPv4 STUN observers and eight candidate
addresses, with no TURN, credentials, DNS or inherited public RTC servers.

Shared traffic observers preserve STEP10 exports and add opt-in pre-close stats
and bufferedAmount peaks for STEP11; default calls still close synchronously.
Normal same-origin TLS/private NSS and old helper signatures remain. Both public
runner modes coexist. The explicit source registry is the exact union of the
accepted 68 and 76 inputs: **85 files**, including `src/lib.rs`, with no dependency
on uncommitted-file discovery after a clean commit.

## Fresh merged results

All source snapshots before/after the affected checks and three runtime runners
match the same current 85 files. After tests, normal-feature native binaries,
browser WASM and generated JS were rebuilt in the independent union target;
native/WASM/bindgen took 38.127/18.009/2.224 seconds. One fresh five-artifact set
was unchanged across all cases. The [receipt](../evidence/browser-rtc-integration-2026-10-04/receipt.json)
records actual per-case executed subsets; harnesses are source inputs.

| Fresh story | Result | Setup, protocol and cleanup seconds |
| --- | --- | ---: |
| STEP10 first WSS materials and critical boundaries | PASS | 31.926 |
| STEP9 default-empty-profile nonempty replica and actual IDB abort | PASS | 23.941 |
| STEP11 closed seed preparation | PASS | 20.212 |
| STEP11 direct, no allowed data relay | PASS | 23.842 |
| STEP11 actual peer UDP DROP, no ordinary handle/fallback | PASS | 29.812 |
| STEP11 matched explicitly allowed relay | PASS | 19.774 |

Each story preserves 120 seconds = 105 work + 15 cleanup. STEP11 preserves its
protected nine-second removal tail, owned five namespaces/four bridges/two
permissive NATs, six captures per case and normal certificate verification.
All five root setup configurators have NET_ADMIN; only NAT/router also have
NET_RAW. UID1000 actors have effective capabilities zero. All final removal and
independent absence checks pass. This fresh normal run includes the four
previously reviewed teardown fixes; it does not relabel the older isolated run.

STEP10 starts with authorization/secret false and no known-material relay
preparation. Exact approval adds one signed grant. Remaining A clock 11824ms
becomes completion remainder 11460ms after deliberate 350ms consumption.
AUTH547ms → generation verified634 → actual secret commit649 → SAVE649 → DONE657
preserves local strict commit before Complete/SAVE and a separate sender
receipt. Same-origin/profile reopen restores identity/materials before new RTC
M/PONG. Pending real IDB abort sends no SAVE; rightful resume keeps the same
grant/request, applied count3→3 and one approval. Different TLS holder claims
receive no approved material. One-use precode survives original native DATA_DIR
reopen with usage/redemptions1/1; an expired original Delivery rejects in1ms
without saving secret, then a new explicit attempt resumes. A different holder's
exhausted new redemption fails. Actual Open-channel loss of one1200B QUIC
datagram makes the subsequent two-second M fail with no handle in2007.7ms;
the exact durable Complete receipt survives failure and same-profile reopen.

STEP9 independently proves BUSINESS and REPLICA M, original stable IDs/bindings,
current permissions and initial/newer release ACKs. Real PONG and original
signed36B header/body save/reopen pass. Legitimate saved source revision1→2
retains effective permission1→1. Targeted real transaction abort yields no
Complete or durable Receipt and leaves the target absent after reopen while
baseline material remains. An intentional initial mDNS join probe fails before
the owned multicast route makes its follow-up pass; not every setup command0.

STEP11 uses one immutable **closed** prepared seed cloned separately for direct,
blocked and relay. That allowed-relay preparation is not first-material proof.
Direct BUSINESS and independent REPLICA have their own current M proof and
initial/newer ACKs before data, actual PONG and the same original36B save/reopen.
Native selected local private host/browser-selected WAN srflx, actual STUN
recvfrom observations, all six capture points and both conntrack translations
agree; both selected pairs use nonrelay UDP. BUSINESS payload window has WAN
12/14 packets (1476/1614 IPv4 bytes, no STUN); independent replica setup/save
67/69 (16153/21551 bytes, seven STUN packets each direction). Encrypted outer
captures alone cannot identify STREAM; these windows correlate separate SDK
and per-M QUIC observations. All normal captures have zero kernel drops and no
packet-limit exhaustion. Actual peer DROP produces no ordinary release/handle,
DC bytes or relay/TURN fallback; original caller12019.5ms, subsequent native
cleanup observation0.638ms, Complete retained. Relay has zero physical RTC/peer
UDP and actual endpoint relay TCP. Socket counts are measured baseline
quiescence (blocked5→5), not global socket zero.

## Finite cost and validation limits

| One matched run | Direct RTC | Allowed relay |
| --- | ---: | ---: |
| Member setup ms | 2204.362 | 501.534 |
| First PONG from client connect ms | 2676.927 | 1064.648 |
| Save-inclusive36B ms | 2408.523 | 641.226 |
| Save-inclusive body B/s | 14.947 | 56.142 |
| Native CPU seconds | 0.711606 | 0.532687 |
| Observed Chromium CPU seconds | 3.99 | 3.48 |
| Simultaneous sampled Chromium RSS bytes | 1296982016 | 1285779456 |

Five steady member RTTs are about15–16ms direct and17–19ms relay. RTC is slower
in this run. One ordered finite36B comparison on a shared host cannot establish
sustained throughput, WAN/NAT-class success rates or a performance benefit.
Native wait4/exec HWM, sampled browser PID/starttime descendants, service/driver
costs, queues and wire layers remain separate. Unsampled final ticks and
short-lived processes may be missed, shared RSS is counted repeatedly, kernel
memory and unknown SCTP/copy costs remain unmeasured.

All26 affected-check commands exit0: native current/default, workspace libraries,
public admission/resource/profile/current-M/store/replica, WASM rtc/default,
format/syntax, real STUN socket and evidence parser. Native307 passing entries
are123 workspace libraries+2 fixture CLI+182 relevant target entries, including
inert crash workers; one pre-existing ignored p2panda test remains. Actual owned
MQTT runs the holder/public-resource branches once in their full targets, with
broker removal and absence verified. No unchanged Noq suite, slow NAT matrix,
Go/OpenSpec or stopped IRC branch was replayed.

The first integration typecheck's actual String/&str error is retained separately
from protocol failures; `.to_owned()` fixes the fixture relay resource. No source
change follows final checks/runtime. The new evidence package keeps original,
stored-raw and deterministic gzip hashes. All24 pcaps strip payload/ICE
attributes while retaining actual headers, timestamps and wire lengths; they
are explicitly not byte-identical original captures. Ordinary JSON nulls and
numeric process exit codes remain. Private invitation/code/description strings
and raw fixture keys are not published. Earlier accepted evidence is unchanged.

Unknown-history recovery, full rotation/migration, lifecycle/device parity,
UI/IWA, default public facilities and TURN adoption remain separate scopes.
See the [integration evidence index](../evidence/browser-rtc-integration-2026-10-04/README.md)
for exact runs, lineage and independent review state.
