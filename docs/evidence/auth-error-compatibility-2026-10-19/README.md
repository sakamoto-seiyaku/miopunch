# Auth error compatibility evidence

MIOPUNCH-96 restores the original exhaustive group error and process result, while policy-checked processing returns an independent error. Implementation: `e240107d3f805a0b18ad1c2979c7e4b0c4ccc922`, baseline `a6cb8adb19dfe3b8cf95076c6201041dc6ed5163`. See [implementation note](../../notes/2026-10-19-auth-error-compatibility.md).

- [Validation](validation.json): fixed unchanged official/fork consumer, fork E0004, repaired runtime, exact enum/lock continuity, 21 affected test entries, final Native/WASM builds and three current SDK scenarios. One entry is the existing subprocess worker, also exercised by its process-restart parent.
- [Independent review](review.md): separate Standards and Spec reports with zero outstanding findings.
- [Raw verification](raw-verification.tar.gz): drivers, selectors, original Rust sources, pre-edit source copies, compiler diagnostics, actual selected inputs, affected test logs and SDK scalar receipts. Every member is read back and checked against [its manifest](archive-manifest.json).

The official fixture uses unchanged Rust sources and only supplies standalone workspace metadata; selectors preserve complete locked dependency rows. The original upstream snapshot has no lock. Root Cargo.lock stays unchanged. The same old consumer is committed as permanent compatibility coverage. Both error paths use one algorithm; checked policy still precedes graph insertion at the causal predecessor. Native quarantine and Browser explicit pending classification remain distinct.

SDK scenarios cover real approval, durable authorization/secret delivery, persistent identity reopen and member PING/PONG; disabled/missing RTC, cancellation/deadline/local-departure negatives; and Browser incoming manual approval/precode/retry/replica plus original-member RTC improvement. Actual SDK/WebSocket/actor cleanup precedes context disposal and removal of generated keys/NSS. A peer without the departure record keeps its independently evaluated current authorization view.

Private SDK identities, SDP, stores and raw SDK logs remain local. Published SDK results use an explicit scalar/hash allowlist. Binaries/WASM and full selected source copies remain at local receipt paths. Fourteen obsolete intermediate M93 test executables are losslessly compressed with exact original hashes and restore paths; all SDK pins, final tests, official baselines, raw logs and inputs remain. Failed pre-Cargo preparation and zero-change cleanup reviews are retained honestly.

This qualifies the affected M96 flows on owned loopback facilities, without replacing the public/physical matrix or closing broader M91 work. All 249 pre-existing WIP files retain their bytes; no push is performed.
