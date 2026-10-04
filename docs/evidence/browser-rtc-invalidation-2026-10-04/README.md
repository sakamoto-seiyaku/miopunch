# STEP8 RTC invalidation evidence

Four independent standard browser/native stories passed. Independent Standards review found 0 hard violations and 2 nonblocking P3 judgments; Spec review found 0 findings and accepted the scoped STEP8 evidence. Dispatcher acceptance follows postcommit verification and the full handoff. This package does not claim a successful controlled delayed-notification experiment or completion of the eleven-step program.

| Story | Total fixture/protocol seconds | Result |
| --- | --- | --- |
| Explicit attempt cancellation, then distinct new M/RTC succeeds | 9.029 | PASS |
| Actual signed SDK member removal and browser applied record | 7.555 | PASS |
| Actual durable native current-credential generation install | 7.656 | PASS |
| Actual installed RTC carrier resource disable | 7.696 | PASS |
| Public unified entry, resource story | 7.771 | PASS |
| Normal WSS-started M, new release ACKs and retained data beyond setup | 41.335 | PASS |

The four standard stories share all 52 recorded relevant sources and the same five executed artifacts. The later public wrapper has a separately recorded one-file source delta and its actual resource receipt. The focused 72-test native suite, all-targets/WASM checks and normal WSS run use the same production Rust and executed artifacts; their subsequent three driver-only deltas are explicit in `receipt.json`.

`receipt.json` includes full source/artifact identities, safe story observations, actual commands, cleanup receipts, and an archive manifest. Archives retain exact raw log/JSON bytes except the normal WSS story: its 22 `usernameFragment` fields are recursively omitted from durable RTC stats. That entry records the original raw SHA-256, transformation, archived-content SHA-256 and compressed SHA-256 separately; it does not claim byte equality. Native component tests use actual authenticated QUIC and an owned queue bridge; they are explicitly separate from actual browser RTC qualification.

The standard secret story changes only native durable current credential/source: generation 0→1/current pin true, membership and resource unchanged, browser local generation 0 preserved. Browser attempts a new exact-path release probe 9→16, does not acknowledge it and returns no normal handle. Native sends freeze after actual source retirement. Browser's extra 30-byte validation packet occurs before its local retirement; physical sends freeze after local retirement and through late events. Immediate native socket count 8 falls to baseline 6 in final/late observations.

Late events use captured actual installed callbacks and actual closed-PC full SDP. Rust closure disposal rejects the captured callbacks; full SDP application rejects with `InvalidStateError`. The receipts do not pretend a disposed Rust handler ran or claim a trickle ICE implementation.

Historical outcomes remain distinct:

- `secret-stale-final-red`: genuine standard runtime bug, returned a stale business handle after real SDK generation installation.
- `secret-early-close-green`: no handle, but peer close arrived before the new probe; not causal proof of the guard.
- `secret-controlled-cleanup-prerequisite-fail`: native M/IO retired and real channel remained open, but browser M/PC already closed; the required browser-live prerequisite failed. Its full assertions remain unchanged in the diagnostic `secret` mode of the narrow runner. This is not a standard story result.

The verbatim independent reports are [Standards](standards-review.md) and [Spec](spec-review.md), against accepted STEP7 base `221950480e92e8b4b29515a74566ccab5693e0a1`. The Standards judgments concern a separate typed checkpoint-report ownership key and shared endpoint setup for two tests. They remain documented optional maintenance: actual lease/original-M checks constrain releases, and the small separate public-seam fixtures have no observed correctness defect. No production refactor or runtime replay was made solely for these judgments.

Each actual owned fixture has stop-work 45 s, cleanup reserve 15 s, total 60 s, unchanged product caller deadline and empty STUN/TURN lists. Normal TLS/private NSS is preserved. Exact owned processes, container and network are removed, and absence queries succeed. No SDP, ICE credentials, DTLS fingerprints, exporter or secret material contents are retained.
