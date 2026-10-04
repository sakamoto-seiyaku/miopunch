# STEP9 member PONG and nonempty original-record replica

Specification: [WebRTC replan, STEP9](../../notes/2026-10-03-browser-webrtc-replan.md#9-真实成员业务和非空-replica). Implementation and qualification boundaries: [runtime note](../../notes/2026-10-04-browser-rtc-member-replica.md). Fixed source baseline is `78e4f9448135068f2630114210a7576378338a50`; the baseline worktree was clean. This package records STEP9, affected STANDARD STEP8 invalidation and normal WSS compatibility; it does not complete the full eleven-step plan.

[receipt.json](receipt.json) contains the exact 60-source SHA256 snapshot, five executed artifact hashes, final commands/timings and each archive's original raw, archived raw and gzip SHA256. All six final fixtures ran without source changes and with the same artifacts; the 109-test current validation has the same source hashes. Archives use deterministic gzip (`mtime=0`). Production artifacts remain generated local files and are identified by hash, not committed here.

## Final actual runtime

| Archive prefix | Actual scope | Protocol/fixture seconds | Result |
| --- | --- | ---: | --- |
| `member-replica-final` | Public ordinary PING→PONG; independent real nonempty LogSync; exact SDK record save/reopen; targeted actual IDB abort/reopen | 24.284 | PASS |
| `cancel-final` | Actual attempt cancel; old retirement and distinct newly qualified M | 8.912 | PASS |
| `member-final` | Real signed native member removal and browser SDK saved-source application | 7.776 | PASS |
| `secret-standard-final` | Durable native current material install/store pin invalidates old M | 7.634 | PASS |
| `resource-final` | Installed RTC registry/carrier disable invalidates old M | 7.747 | PASS |
| `normal-wss-final` | Existing WSS member compatibility, shorter remaining clock, cancel and ordinary empty replica smoke | 41.666 | PASS |

Each prefix contains original runner results, story JSON and protocol log. Runner results include all five exact cleanup commands, their actual exit codes and queried container/network absence. Story logs include fixture exit 0. Build time is separate from protocol/fixture time. The outer `final-runtime-results.json.gz` records actual command exits, source-before/after equality, same artifact identity and no stopped controlled diagnostic. Its own parent tool returned exit 0.

BUSINESS, successful REPLICA and aborted REPLICA each have their own ALPN, original local/native M IDs, binding/stable equality, initial and new final-release ACK on exact RTC path, and counters. Final native ACK comes from actual `rtc-path` actor events after the new probe; the earlier native snapshot carried in private FINAL is not substituted. Successful REPLICA changes diagnostic source revision 1→2 while per-M permission stays 1→1/current true, closes only after real save/Receipt, and reads back at the same browser origin/profile. The source record is supported signed management Add plus opaque 36-byte body; source is also author, not a separately qualified third-author forwarder.

The failure hooks a real target-specific IndexedDB readwrite transaction before target put/commit. Actual SDK rejection/no complete/native durableReceipt false, exact target absent after reopen, retained baseline header/body and retained Complete identity/material flags are all in the story. This is a storage-commit interruption, not a process crash or network-cut result. Native final association/pump/queue/route counts are zero and socket count is its measured baseline 6; browser's three PCs and owned callbacks/observers are closed/detached.

## Failure history retained

| Archive prefix | Meaning |
| --- | --- |
| `business-red` | Genuine runtime failure: ordinary request echoed PING rather than member PONG. |
| `nonempty-red` | Genuine post-save integration failure: target SDK record was really saved but global snapshot revision retired its otherwise-current RTC M before Receipt/complete. Not a pre-save failure. |
| `nonempty-first-green` | First real nonempty successful exchange after the browser per-M current-source fix. |
| `save-boundary-harness-fail` | Normal save and actual target transaction abort succeeded; derived members-set ordering assertion failed. Post-abort reopen had not yet run. Original FAIL retained. |
| `save-boundary-first-green` | Driver canonical set comparison fixed only after the failed run ended; actual normal/target abort/reopen passed with unchanged five executable artifacts. |

Compile/interface corrections are not labelled product runtime reds. Final sources supersede historical per-run source hashes; each historical receipt retains its own exact provenance. The stopped STEP8 controlled-PC/notification-delay diagnostic remains in the STEP8 package as prerequisite failure and was not rerun or reinterpreted by STEP9.

## Validation and reviews

`validation-results.json.gz` and ten named validation log archives record 109 passed current related native tests: 25 full library, 37 full management/auth/store/native-network/offline-replica/daily-management, and 47 current guard/proof/attempt/wire/RTC-release/invalidation tests. The 13-test full replica target includes the new deadline test and its existing inert crash worker; the focused `deadline` run is additional evidence, not an extra test in 109. All-targets, WASM rtc/default checks, formatting, diff and Node/Python syntax passed. Unchanged slow NAT matrices and old archived Go/OpenSpec gates were not added.

Independent [Standards](standards-review.md) and [Spec](spec-review.md) reviews inspected the actual nonempty staged implementation against the fixed baseline. Both returned zero required findings. Standards recorded one optional duplicated diagnostic path-stat serializer across browser/native adapters; it is deferred to a later observation-schema change to preserve the existing platform seams. No source or runtime delta followed review. The receipt records the actual reviewer identities, runtime model/HIGH readback, report hashes and final metadata audit. Final local commit identity and clean postcommit verification are in the implemented temporary handoff, since a commit cannot contain its own hash.

## Sensitive data and transformations

Story output does not preserve SDP, exporter bytes, secret keys, saved private material or DTLS fingerprint. `secretGeneration` is a counter and `secret_saved` is a boolean; neither is secret content. Existing normal WSS RTC stats included 22 `usernameFragment` and 16 `iceLocalUsernameFragment` string fields. Only that story JSON archive recursively omits those 38 credential fields. Its receipt has `byteExact=false`, exact field counts, original raw SHA256, transformed raw SHA256, gzip SHA256 and the declared transformation. All other archives are byte-exact original files. No sanitized file is represented as the original raw byte stream.

Scope remains known-current materials, browser offerer/native answerer, owned normal TLS/private NSS namespace and empty ICE/STUN/TURN. STANDARD secret invalidation is not complete browser/network-wide rotation or delayed-notification causal qualification; resource disable is that installed RTC carrier, not global configuration. First WSS materials, real heterogeneous-network direct traffic and cost remain STEP10/11; no new default backend, resource, crypto, sync protocol, UI/IWA or TURN decision is made here.
