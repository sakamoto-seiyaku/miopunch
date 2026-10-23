# MIOPUNCH-113 onboarding qualification

The full Native and Browser suites passed on 2026-10-23. Later resource-policy
fixes have separate affected regression receipts and immutable pins. The public
operations and engineering formats are described in the
[operation guide](../../notes/2026-10-23-client-onboarding.md).
[summary.json](summary.json) contains the artifact hashes and scoped results.

| Runtime | Actual qualification |
| --- | --- |
| Linux host ↔ Windows laptop | Full Native graph-six suite: both creator/joiner and pairing directions; 46 native process instances; 16 SDK PING/PONG exchanges, each selecting an IP path. Native Windows PE ran on the actual laptop. |
| Chromium 153.0.8010.12 | Nine independent real browser profiles/WASM identities with IndexedDB and Web Locks; Native↔WASM creation/admission/provider roles; two browser member-stream exchanges; page reopen preserves identity and membership. |
| Cross-administrator continuation | Actual Native and WASM pending applicants finish through another administrator's fresh precode while the original issuer endpoint is closed. Each new code consumes one use; identities and original valid grants remain, with no extra membership record. |
| Resource policy and trusted import | Current Native graph twelve: complete configured MQTT/IRC spelling/default-port and WSS/logical-denial matrix with both `Signaling` and `Lookup`, plus sixteen public join/import cases on both actual platforms; 88 processes exited normally. Actual broker contact and live retirement also pass below. |

Each suite exercised genuine success, early failure and cleanup. Coverage includes
plain/optional encrypted original recovery cards, incorrect password/ciphertext,
failed save before acknowledgment, durable creator gate, full text invitations,
pending-request deduplication, durable rejection, explicit fresh request, repeated
approval, authorization-only interruption, precode limit-one continuation,
offline membership, wrong pairing sender, substituted peer/routes/material,
failed pairing publication/IndexedDB save, exact retry, and ordinary admission
after pairing. Resource cases retain explicit empty replacements, disabled
origins/uses and disabled mapped signaling facilities across reopen; exported
contact material omits disabled resources.

The two administrator fixtures used the existing SDK management/signature APIs
and real replica exchange. This qualifies precode continuation, without claiming
the later MIOPUNCH-116 per-operation genesis-confirmation product flow.

## Fixed artifacts and raw receipts

All paths below are repository-relative. Artifacts are immutable pins in the
shared `.artifacts` store; source archives, selected dependency files and raw
Cargo output stay with each build receipt. Runtime controllers did not compile.

| Graph | Current receipt | Cargo result |
| --- | --- | --- |
| Native Linux, dev/default | `.artifacts/runs/m113-linux-retirement-12/build.json` | Exit 0; 465 Fresh, 2 own targets rebuilt. |
| Native Windows GNU, dev/default | `.artifacts/runs/m113-windows-retirement-12/build.json` | Exit 0; 526 Fresh, 2 own targets rebuilt; pinned compiler-container cleanup 0. |
| Linux technical maintenance probe, same dev/default library | `.artifacts/runs/m113-linux-retirement-probe-12/build.json` | Exit 0; 466 Fresh, only the probe target rebuilt. Uses the existing public maintenance-owner API for verification. |
| WASM, dev/rtc-probe | `.artifacts/runs/m113-wasm-flow-3/build.json` | Exit 0; 295 Fresh, 1 own target rebuilt; bindgen 0.2.129 exit 0. |

The Native graphs retain 424/485 selected depfiles and 286 local selected inputs
each. WASM retains 263 depfiles and 201 local selected inputs, all matching current
selected source. Later Native-only source and build-driver edits are recorded as
unselected WASM deltas; the unchanged WASM graph was reused.

Persistent runtime evidence, with source-graph scope retained:

- `.artifacts/runs/m113-max-20261023/native-final-2/{events.jsonl,result.json}`
- `.artifacts/runs/m113-max-20261023/browser-final-1/{events.jsonl,result.json}`
- `.artifacts/runs/m113-max-20261023/final-controllers-2/`: seven frozen runtime inputs and source archive, reverified after both suites.
- `.artifacts/runs/m113-max-20261023/{native-pair-final-3,browser-pair-final-3}/`: Native graph-five pairing regression and cleanup after the sender guard; controllers retained in `final-controllers-3/`.
- `.artifacts/runs/m113-max-20261023/native-review-full-6/`: later complete Native graph-six suite after private-file helper consolidation and original runner migration.
- `.artifacts/runs/m113-max-20261023/{existing-runner-current-6,existing-runner-old-m112}/`: real original-runner Windows/Linux flow on graph six and compatibility with the accepted M112 Linux pin.
- `.artifacts/runs/m113-max-20261023/browser-review-pair-6/`: affected Native/WASM pairing on Native graph six and unchanged WASM graph three.
- `.artifacts/runs/m113-max-20261023/expanded-resource-import-final-10b/`: complete two-platform resource/import matrix on Native graph ten; frozen controllers in `import-controllers-8/`.
- `.artifacts/runs/m113-max-20261023/lookup-contact-green-11/`: six real Native graph-eleven public-client cases, two owned pinned Mosquitto brokers, actual socket/subscription counts and two verified public discovery replies. Disabled WSS/logical mappings and single-purpose resources make zero discovery contacts; an independently permitted raw lookup remains usable.
- `.artifacts/runs/m113-max-20261023/live-retirement-green-12/`: current eight-process, three-broker check also drives the real public maintenance owner. Both default-port alias subscriptions retire, later queries avoid that broker, another broker remains subscribed and queried, and reopen retains denial while the other broker returns a verified actual reply.
- `.artifacts/runs/m113-max-20261023/retirement-resource-final-12/`: complete current two-platform matrix with both resource purposes; sixteen imports, 88 process exits 0, Windows forced 0/remaining 0 and private exports removed. Eight controller hashes in `retirement-controllers-10/` match after completion.
- `.artifacts/runs/m113-max-20261023/final-checks-12/`: twenty owned Rust files pass formatting, seven build-driver/cleanup regression checks pass, Python sources compile and `git diff --check` passes.
- `.artifacts/runs/m113-max-20261023/final-runtime-summary.json`
- `.artifacts/runs/m113-max-20261023/browser-preconditions/`: missing shared source, missing browser asset, lock drift, incorrect bindgen hash and newly omitted asset all abort before Cargo.

The completed native suites report every process exit 0, Windows forced stops 0 and
remaining owned processes 0. The browser suite reports all nine contexts closed,
controller exit 0, reader ended and owned process group absent. Private exported
cards, invitation/precode/pairing files and test passwords were removed. Private
durable diagnosis state remains only in the owned temporary runtime directories;
it is absent from these receipts and Git.

## Retained failures and corrections

The retained old browser package lacks the new create/join/pair APIs in a real
Chromium red check (`browser-api-red.json`). The first current WASM compilation
reported child-module visibility errors; those raw logs remain in
`m113-wasm-flow-1`. Subsequent builds reused the unchanged dependencies.

A real Native CLI regression showed that failed file export had prematurely
enabled saved acknowledgment (`native-save-red-1`). File export now records its
marker after the private file is flushed; the full two-platform suite checks
acknowledgment still fails after unsuccessful export.

Browser reverse pairing reproduced an immediate rebind before Web Lock release
(`browser-pairing-1`). Explicit close now waits for actual lock release. Both
directions and same-identity finish passed after the fix.

Windows pairing reproduced a failed-save retry blocked behind unfinished serial
incoming control handshakes. Fixed-stage diagnostic logs are retained under
`pair-save-debug-confirmed`; the minimized old sequence failed immediately and
then waited about 30 seconds. The receiver now has four bounded handlers and
serializes publication only. The same immediate retry completed in about 0.14
seconds (`pair-save-fixed-1`), followed by the full two-direction suite passing.
Temporary diagnostic instrumentation was removed from production source.

A public Native CLI red check also delivered an invitation with another
canonical Create under its own contact (`native-invite-anchor-red`). The sender
now checks its exact root/Create/contact, matching the browser API. Native graph
five reran both Native directions and Native↔WASM pairing, including
this early failure and a genuine changed-wrapper delivery that reaches the
receiver. The later complete Native graph-six suite and affected Native/WASM
pairing check cover the shared private-file helper change; later resource fixes
have their own scoped regressions. The machine summary distinguishes these
builds and results.

Independent review expanded a disabled-mapping fallback finding across binding,
export, import, explicit override, retirement and recovery discovery. Exact-URL
filtering first missed DNS-absolute/default-port spellings. Import then discarded
denied mapping metadata when its explicit native port differed from a local
replacement. Actual public joins reproduced raw MQTT contact and a raw IRC
startup error (`review-import-metadata-red`). Trusted mappings are now retained
for reservation independently of positive permission, with conflict validation
and all local restrictions preserved.

The expanded review also found that generic native-URL permission let a resource
with both `Signaling` and `Lookup` reopen a disabled mapped broker through raw
discovery. Real Mosquitto observed the forbidden discovery subscription
(`lookup-contact-red-10`). Shared raw-resource selection now excludes mapped
origins before binding, export, startup responders or recovery queries contact a
broker. The graph-eleven socket check confirms zero denied contacts while actual
public discovery replies still work for a separately permitted raw origin. No
legacy raw MQTT service-tag bytes changed.

The whole origin-comparison audit then exposed the remaining live-retirement
boundary: distinct legacy wire keys had been compared as policy identity. The
actual public maintenance owner left one alias subscription and made another
query to the disabled broker (`live-retirement-red-11`). Retirement and the
stopped-listener guard now use physical-origin comparison, while wire keys and
unrelated origins retain their contracts. The graph-twelve regression observes
zero remaining alias subscriptions and zero later disabled-broker query contacts;
an independently allowed broker is still queried and remains usable after reopen.
The small `client_resource_probe` is a bounded verification host for this
existing lower-level SDK API; product onboarding uses `NativeClient`.

Experimental controller failures are retained as well: the first browser
continuation run confused node close with controller shutdown. The corrected
host passed the full continuation scenario. A sandbox socket-bind refusal is
recorded separately from the actual host-level Native red check.
The expanded resource fixture initially used an unsupported implicit raw IRC
positive control, then excluded every bootstrap relay from a Windows import
attempt. Both failures and cleanup remain retained; the corrected control uses
the supported explicit raw port and explicitly grants the offered bootstrap
relays while preserving its signaling replacement and denials.
A later both-purpose fixture compared serialized resource-use sets by input
list order. The retained failure (`import-purpose-order-fixture-failure-11`) is
corrected by set-equivalent comparison; no SDK policy changed for that assertion.

## Scope and provenance

Native IP qualification used the available VPN/Tailscale environment. SSH only
deployed/controlled clients and transferred private invitation material; SDK
business exchanges used their actual selected paths. This is a functional
Windows/Linux qualification, without a VPN-free WAN/NAT-matrix claim. Browser
member traffic used the permitted relay. The unchanged slow NAT matrices and
MIOPUNCH-111's specific Edge/VPN RTC problem were outside this run.

The pre-transfer `/tmp` refresh erased earlier raw receipts and private runtime
state. Their reported outcomes are not represented as retained original
evidence. `.artifacts/runs/m113-model-switch-20261023/` records that limit and the
surviving pins. The fresh builds and complete runtime receipts listed above are
retained persistently. A later graph-nine matrix also hit an isolated Windows
startup OS-error-5 before readiness (`expanded-resource-startup-failure-9b`),
matching the previously reported M112 symptom. The failed run and cleanup are
retained; subsequent graph-ten and current graph-twelve matrices passed. Its
root cause remains unexplained.

No product UI, Android lifecycle qualification, credential custody/recovery,
service ACL/guests, per-operation genesis confirmation or successor business
module is claimed here.
