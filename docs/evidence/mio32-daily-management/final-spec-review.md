# MIOPUNCH-32 Spec review

Required findings: **0**. Optional findings: **0**. No missing, partial, incorrect, or out-of-scope behavior found in this slice.

Reviewed final cached WIP against `8ee52120b6499b5ff70142aba571e2bc43c7ce2d`; the reference resolves, diff is nonempty, and there are no subsequent commits. Spec sources: complete MIOPUNCH-32, preserved parent MIOPUNCH-14, native relation/dependency MIOPUNCH-31, ADRs 0003/0007/0008.

`src/daily_management.rs:146` preflights genuine node/root signatures through the causal verifier. The root key plus canonical Create are required; local session state never supplies remote authority. Session ownership, expiry immediately before signing, explicit exit/end, foreign sessions, and reopen invalidate local privileged signing. Administrator demotion preserves Read; separate removal ends membership. Receipt and replay retain original-author verification.

`tests/daily_management.rs:437` exercises five directly signed unauthorized administrator actions through local ingest, actual admitted LogSync receipt, quarantine, reopen, and third-party forwarding. `tests/daily_management.rs:539` covers both real wire orders of concurrent demotion/removal, correction, duplicate application, and reopened views. Historical valid root actions survive window exit and original-author shutdown.

Continuation check: each selected action starts a fresh log with verified causal heads (`src/daily_management.rs:197`). Quarantined unauthorized predecessors cannot enter those heads. Valid-but-filtered records retain applied history and remain served/replayed (`src/management_store.rs:618`). Arbitrary raw continuations referencing an invalid causal operation remain unverifiable; no supported authoring path requires them.

Exact changed source/harness SHA-256:

| File | SHA-256 |
| --- | --- |
| src/daily_management.rs | 8d698590ed1bd3786fe1297d92ade4fac39d7ef75373cafa0d378ee785c77cad |
| src/genesis_recovery.rs | a845ee1a9397fb8c94316390b36416a55de6037fbc044d90008597d744c8f25c |
| src/lib.rs | 5c6ad2826f9673c95a4528a77125855daed7e89d99966cf22624f8982e512d37 |
| src/management_store.rs | 1c4e0ae4c71e0d93e0fec3f69ef8019efd7a9e9c0f03f8fa75192ddfb63876d9 |
| src/native_network.rs | 8ad677d8da73a53f512f3617435daed069d8d8019a971f7025a8509372ecff36 |
| tests/daily_management.rs | 6241ceb9694b1b9c89375e98a6badee995ac6d0a2d6e0174eda080d439a800d9 |
| scripts/check_daily_management.py | 5cb914fb721c7a357e9ecee033893dcf6c394a94c7a7aa106ec8085815a540ce |

All 26 cached/working hashes match `/tmp/mio32-plane/reviewed-staging.json` (SHA-256 `5e7f557aa44c6cfb16b5c24b07db1b1eb6f330b5843c09a2faa21bf4ba52c4f0`); all ten generation-source hashes and 59 protected hashes match. All 13 gzip logs verify original/compressed/roundtrip hashes. Actual suite totals: 274 passed, 0 failed, 4 existing ignored; six management tests pass; MQTT capture reports 92 publishes; five recorded checks exit zero; cleanup readback confirms owned broker absent.

Limits: read-only review of existing native Linux loopback evidence; no tests rerun. Browser evidence is compilation only. No broader NAT/platform, power-loss, formal UI, or deferred lifecycle claims.
