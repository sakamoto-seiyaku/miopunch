# STEP6 Spec review — initial required finding

Reviewer: `/root/webrtc_s6_sol61_high/s6_spec_sol61_high`; parent-specified model `gpt-6.1-sol`, HIGH effort.
Fixed base: `ca3be8bad6887c96022297081ddeafa2e2514364`; actual reviewed HEAD: `a9723c90ab6407d71c9a30a68e8b1a4035935421`, plus all working changes and all five untracked sources/tests.
Read-only review; no source/index/commit/Plane mutation and no builds/tests.

## Required

**P2 — Joint legacy relay capability is negotiated but rejected by the browser.** `browser-probe/src/network.rs:531` unconditionally returns `temporarily unreachable: browser member data path unavailable` after capability exchange whenever `paths.rtc` is false. A saved-member M starting on MQTT, negotiating strict v2 with an older peer, and with an allowed relay on both sides therefore fails even though `paths.relay == true`. Native `src/message_bootstrap.rs:245` retains the existing same-M relay-validation coordinator, but the browser never participates in its relay candidate/status stream.

The actual STEP6 spec says: “旧端无RTC能力且没有共同获准data path，明确不可达” (`docs/notes/2026-10-03-browser-webrtc-replan.md:135`): unreachable is conditioned on the lack of a joint allowed data path. The accepted read-only handoff additionally states: “Preserve legacy v2 behavior when a joint allowed relay/IP path exists.” Add a bounded browser continuation of the existing native relay-validation exchange on the original restricted M, using the same caller deadline and current qualification, and cover an owned legacy-plus-allowed-relay story. This is initial path establishment, separate from STEP7 RTC repair.

## Other categories

No other missing/incorrect requirement or unasked behavior found. Optional findings: zero.

Exact live route qualification is rechecked by queued final authorization; both hooks withhold ordinary handles until fresh probes and private confirmation; successful weak per-M owners support Business and Replica without a setup-clock lifetime. Earlier actual WSS receipts were examined as supporting evidence, not final-source qualification. Owner final native/runtime/hash receipts remain pending and were not substituted with reviewer tests.

Available for same-agent focused recheck of the required fix.
