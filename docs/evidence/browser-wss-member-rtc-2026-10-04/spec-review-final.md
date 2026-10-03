# STEP6 Spec review — same-agent required-fix recheck

Reviewer: `/root/webrtc_s6_sol61_high/s6_spec_sol61_high`; parent-specified `gpt-6.1-sol`, HIGH effort. Fixed review base remains `ca3be8bad6887c96022297081ddeafa2e2514364`; reviewed StageA HEAD `a9723c90ab6407d71c9a30a68e8b1a4035935421` plus WIP/untracked sources. Read-only: no source/index/commit/Plane edits, builds or tests.

## Result

**Zero remaining required findings.** Original P2 is resolved. Optional findings and unasked behavior: zero.

The STEP6 failure requirement is “旧端无RTC能力且没有共同获准data path，明确不可达” (`docs/notes/2026-10-03-browser-webrtc-replan.md:135`). `browser-probe/src/network.rs:631` now handles negotiated joint relay capability through `legacy_relay_continuation` (line308), continuing the original restricted M with the original hook deadline. Browser qualification rechecks current persisted material, membership/proof, and effective relay-origin permissions during validation and after bilateral confirmation; disabled origins are excluded from advertisement. The shared `src/member_path_protocol.rs:34`/`:78` exchange preserves the existing size-prefixed candidates (at most4, at most1024 bytes) and strict one-byte bilateral status. Native and browser use opaque fresh path probes, rather than inferring ACKs from passive counters. No second M, vendor API, repair engine or setup budget was introduced.

The actual protocol red `/tmp/mio-rtc-s6-wss-legacy-relay-red-c-20261004` records strictv2, jointrelay=true, RTC=false, only MQTT initial path, followed by the original browser rejection. The final fresh-build/runtime green `/tmp/mio-rtc-s6-wss-legacy-relay-qualified-20261004` passed in2.871 protocol seconds: both fresh ACKs/watermarks, unchanged local stable/exporter comparisons, original private stream3 continuity, normal same-M relay STREAM growth, no RTC association, and durable Complete. The owned dormant WebSocket fixture permits no fabricated open or relay packets before real MQTT capability negotiation; cancellation cannot reopen it.

All53 source hashes in the final receipt match the currently read files; sourceBefore equals sourceAfter. Five native/pkg artifact identities are recorded, and owned broker cleanup exits0. Native passive received-ACK counter0 is not an ACK-proof failure: the opaque probe reports the actual authenticated ACK.

This focused closure does not substitute for pending owner qualification of other final-source cases or later STEP7–10 work. Available for the same-agent final docs/receipt check.
