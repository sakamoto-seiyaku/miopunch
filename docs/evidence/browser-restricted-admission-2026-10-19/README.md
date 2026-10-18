# Browser restricted admission evidence

MIOPUNCH-94, based on completed M93 commit `e697315e`. Implementation:
`ece50f3b1711f85c4beead1bffd31f2ca3e03733`.

The actual Browser SDK uses the restricted endpoint/admission contract while
preserving its existing wire and storage format. The before-edit SDK story and
JavaScript consumer were fixed before production changes. That same consumer
passes with the migrated SDK and the immutable M93 Native counterpart.

| Final check | Result |
| --- | --- |
| RTC-enabled WASM build | Pass; 3,162 selected inputs |
| Default browser compiler check | Pass; 3,150 selected inputs |
| Affected Native shared-library compiler check | Pass; 6,577 selected inputs |
| Fixed SDK consumer: request, one approval, durable materials, both identities reopened, member PING/PONG | Pass |
| Existing Native/Browser member Relay→RTC improvement | Pass on the original member |
| RTC initial release and affected failure cases | Pass |
| Browser serving manual admission, precode/retry, member streams and replica | Pass |
| Browser incoming active-control/RTC improvement | Pass on both original browser members |

The five final SDK cases overlap in setup and are not five independent protocol
implementations. Every case uses fresh synthetic identities and owned loopback
facilities. Active improvement explicitly selects `PreferDirect` in its fixture;
the default path preference is unchanged.

Negative cases observe no local ordinary handle when RTC APIs are missing,
resources are disabled, a pending unqualified channel is cancelled or expires,
or public local departure occurs after qualification and before final release.
Retired originals, routes, queued bytes, associations and pumps are checked.
The fixed Native counterpart has not received the browser's departure record;
its independent release is recorded separately under the accepted propagation
window. This does not assert instantaneous remote revocation.

SDK close is observed before context destruction. Initial and later WebSocket
close observations are retained, including each extra browser context. Native
and relay processes exit naturally, containers are removed, and generated
private test keys/NSS data are removed only after actor absence is verified.

`validation.json` contains allowlisted outcomes and hashes of private local
receipts. `raw-verification.tar.gz` retains controllers, pinned lock/selectors,
build contexts, actual compiler output, dependency-input receipts, execution
preconditions, all runtime exits including failures, and cache cleanup review.
Every archive member was read back and checked against `archive-manifest.json`.
Full pinned SDKs and selected-source copies remain in their local receipt paths.
Private runtime logs, generated identities, SDP, stores and keys are excluded.

Intermediate failures are retained: the canonical migration compiler red, a
control-exchange diagnostic method mismatch, the old builder admission-gate
conflict, negative-fixture close/restart mistakes, a revocation assertion that
conflated the two authorization views, and an immediate-only extra-context
socket observation. Failed runs are not counted as qualified passes.

The original 249 workspace paths remain byte-for-byte unchanged. Cache cleanup
removed only three individually reviewed obsolete test executables after
checking 35,963 published evidence files and excluding published references,
libraries, fixed SDK artifacts and normal binaries. No target tree was copied
or reset; no dependency lockfile was re-resolved.

Review results are in [review.md](review.md). Public/physical platform matrices
and Android throughput/power checks remain outside this evidence scope.
