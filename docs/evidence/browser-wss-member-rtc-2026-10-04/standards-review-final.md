# STEP6 Standards focused final-source recheck

Same reviewer: `/root/webrtc_s6_sol61_high/s6_standards_sol61_high`, parent-specified `gpt-6.1-sol` / HIGH. This supplements `/tmp/miopunch-s6-standards-review.md`; fixed base remains `ca3be8bad6887c96022297081ddeafa2e2514364`, StageA HEAD `a9723c90ab6407d71c9a30a68e8b1a4035935421`.

Reviewed new legacy-relay deltas: shared RelayValidation, native/browser fresh opaque probe and same-M continuation, effective browser relay permissions, diagnostics bounds, and owned legacy-relay fixture. Applied the same documented repository standards and complete labelled smell baseline. No source/index/Plane edits or builds/tests performed.

## Required findings

**0.** No new documented-standard violation established. RelayValidation bundles the private stream and its continuity evidence, preserves strict existing legacy candidate/confirmation wire bounds, and shares that protocol between adapters. The original caller's outer remaining budget still bounds setup. Browser relay candidates exclude disabled origins and permissions/materials are checked again before release. Closed diagnostics are pruned against the existing 32 concurrent member-proof bound. The exact owned relay remains CONNECTING without a socket or fabricated open/data events until the actual MQTT capability event; cancellation/close prevents late opening.

## Optional judgement calls

**No new smells warrant a separate finding.** The original optional **possible Duplicated Code** in browser RTC Control/Ordinary offer/profile/probe phases remains intentionally parked: `JsFuture::from(owner.association.pc.create_offer()).await?` and `phase(&mut recv, "probe-ack").await?` recur at browser-probe/src/rtc_member.rs:252/:461. Their release and lifetime tails intentionally differ; this is not a blocker.

Readback: the fresh owned legacy-relay qualification at `/tmp/mio-rtc-s6-wss-legacy-relay-qualified-20261004/results.json` now reports `passed: true`, `sourceUnchanged: true`. Its actual member receipt records fresh opaque probe ACKs and private-stream continuity before ordinary release. This result covers that one legacy-relay story; broader final qualification remains pending.

Total remains **0 required findings, 1 parked optional smell**. Available for a same-agent narrow final documentation/receipt readback.
