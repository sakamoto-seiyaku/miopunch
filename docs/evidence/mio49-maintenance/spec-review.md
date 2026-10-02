# MIOPUNCH-49 independent final Spec review

Reviewed the complete comparison
`4185f55caf70bf884a90cbde806d207caf582dba...ecb6ea9a9c57128df1c31a694823e49eaa11ba08`
(commits `5a780dbc` and `ecb6ea9a`) against actual MIOPUNCH-49 and parent
MIOPUNCH-14, the current design baseline, recorded integration boundaries, and
the final/post-review evidence. Spec axis only. **Zero confirmed unresolved
specification findings; no confirmed scope creep.**

The [initial review](spec-review-initial.md) identified two P2 deviations. Both
are resolved in the reviewed implementation:

1. MIOPUNCH-49 requires “连续失败后退避” (line 9 of the saved Plane description).
   Maintenance now uses `ResourceEndpoint::connect_for_recovery`, preserving
   accumulated failures across successful QUIC handshakes. The coordinator
   resets pacing after complete, authorized, durable recovery; incomplete or
   failed recovery still records failure. The actual authenticated-peer regression
   first succeeds at LogSync, then sends an invalid manifest after real TLS/QUIC.
   [RED](review-failure-red/results.json) observed handshake intervals
   1739/1396/1395 ms; [GREEN](review-failure-green/results.json) observed
   1720/2407 ms, confirming progressive failure pacing rather than handshake
   success erasing recovery failures.

2. MIOPUNCH-49 requires “所有入口失败时持续可解释等待/退避” (line 18);
   parent MIOPUNCH-14 requires “得到暂不可达状态” (line 152).
   Existing-neighbor sync failure now sets `TemporarilyUnreachable`, and an empty
   neighbor view no longer relies on historical `last_sync` being absent before
   reporting waiting. The publishing fallback also corrects stale `Connected`.
   Historical source/scope completion remains available. The same real regression
   observed `stale_connected=true` before the fix and `false` afterward.

[Post-review verification](post-review/results.json) passes all twelve ordered
acceptance cases, 26 resource checks, 12 recovery checks, all-target typechecking
and formatting. Its repeated failure check independently records 1699/2379 ms
and `stale_connected=false`. Earlier unaffected short/library/WASM evidence
remains in [final-suite](final-suite/results.json).

These results qualify the recorded native Linux controlled environment and
supported carrier profile. No source edits, new tests, or unchanged slow NAT
reruns were performed by this reviewer.
