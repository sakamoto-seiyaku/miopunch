# Necessary Answerer API capability check

The SDK now supports both RTC roles, so its runtime capability check must require
`createAnswer` as well as `createOffer`. The previous check advertised support when
the real Browser API lacked `createAnswer`. The correction adds that one necessary
method and updates two constructor-denial unit fixtures to expose the complete
method shape. A new unit case checks that missing Answerer support is rejected
before creating a PeerConnection or DataChannel.

The actual SDK reproduction uses owned Chromium and the formal
`BrowserNetwork.start`, `status`, `path_diagnostics` and
`rtc_invalidation_diagnostics` interfaces. A policy-off startup first saves an
owned identity/reservation. After closing/freeing that instance, the controller
deletes the real `RTCPeerConnection.prototype.createAnswer`, verifies it became
undefined, and reopens the same origin with RTC permission enabled. Other methods
remain available. The object observer stays in `observe` mode; the separate
`injection` receipt records the real method deletion.

| Observation | Before correction | After correction |
| --- | --- | --- |
| Raw SDK test exit | 1, expected capability assertion fails | 0 |
| Actual support / permission / effective | true / true / true | false / true / false |
| Fixed reason | available | api-missing |
| PC / DataChannel created | 1 / 1, both closed | 0 / 0 |
| Retained RTC associations / pumps | 0 / 0 | 0 / 0 |
| Same saved identity and status flags | true | true |

Both cases retain a running `AwaitingApproval` node. Authorization and secret were
absent before and after; this case qualifies an owned persistent identity and
reservation, and does not qualify preservation of approved member materials or
member communication. No Native member process, admission exchange, role/ACK
loop, physical device, public facility or STUN probe ran. Startup caller limits,
permission, identity, role protocol and budgets remain unchanged.

Each case uses only owned local TLS/WSS MQTT and temporary private CA/NSS trust
with certificate verification enabled. SDK close/free, immediate and 100 ms late
RTC/object observations, WebSocket retirement, database deletion, browser-context
closure, empty local socket sets, server closure, owned-container removal and
private trust/profile directory deletion complete. The existing real failure is
preserved byte-for-byte. Global browser trust was not changed.

The direct Node boundary invocation first reports five tests with four passing
and the new missing-Answerer case failing; it then passes all five after the
correction. The first `node --test` invocation produced a file-level harness
failure without the counterexample details. Its log is retained separately and
is not treated as SDK or API evidence.

The affected WASM build and bindgen both exit zero. Actual dep-info lists 187
current WASM inputs, with copied source snapshots retained at the frozen build
path. All package snippets are hashed: the capability snippet changes to
`666a8580…`, while processed WASM/glue remain byte-identical. Native binaries are
reused only after checking all 276 actual compiled Native inputs during this
window; no Native build or new Native qualification is claimed. Later edits by
the Native fixture owner fall outside these captured input matches. This is not
a whole-repository or later-current-commit source qualification.

[receipt.json](receipt.json) pins 27 byte-exact compressed originals, the SDK
RED/GREEN results, local test outcomes and the existing root proof reference.
[source-input-map.json](source-input-map.json) records actual input hashes,
dep-info, source-copy locations and artifact/snippet hashes. Large WASM, ELF and
source-copy trees are referenced without duplication. Exact temporary controllers
are under `controllers/`; raw records are under `raw/`. This subtree contains no
certificates, private keys, browser profile or durable membership secrets. The
previous reviewed `f8f42c1b` batch remains unchanged and outside this review.
