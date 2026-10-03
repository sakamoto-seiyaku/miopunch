# STEP6 Standards review

Reviewer: `/root/webrtc_s6_sol61_high/s6_standards_sol61_high`; parent-specified model `gpt-6.1-sol`, effort HIGH.

Scope: `rtk proxy git diff ca3be8bad6887c96022297081ddeafa2e2514364`, including StageA integration HEAD `a9723c90ab6407d71c9a30a68e8b1a4035935421` and all five untracked files. Commit list contains the StageA integration commit. Standards: AGENTS.md/RTK.md, CONTEXT.md, docs/agents/domain.md, current design/ADR0004–0006, and the full code-review smell baseline. Repository overrides applied; tooling-enforced formatting omitted. No source/index/tracker edits or builds/tests performed.

## Required

None. No documented-standard violation established. The finite protocol/version/role types, shared caller metadata, exact owned Control seal with a final current-qualification check, and weak retained native/browser holders preserve the current design boundaries. IRC's measured stop branch remains explicitly unqualified; its receipts do not claim STEP6 success.

## Optional judgement call

**Possible Duplicated Code — browser-probe/src/rtc_member.rs:252 and :461.** Control `bootstrap_control` copies the Ordinary `rtc_member_attempt` sequence for offer creation, answer binding/profile validation, route installation, channel opening, fresh probe ACK and bilateral retention. Both contain `JsFuture::from(owner.association.pc.create_offer()).await?`, `phase(&mut recv, "probe-ack").await?`, and `owner.lease.retain(...)`. Consider extracting that shared protocol prefix so future binding/profile/probe changes cannot drift. Keep the Control/Ordinary path-opening, release, continuity and retained-lifetime tails distinct; their differences are intentional. Existing Association/OwnedRtc/pump/wire/profile reuse substantially limits this concern. This is optional maintainability work, not a release blocker.

Readback: both pre-final actual WSS results are green and include the executed `browser_wss_member_native` binary hash and `sourceUnchanged: true`. The later owned setup branch at scripts/check_browser_rtc.py:133 appends that hash; the initial omission concern was retracted. Those results precede final source/format deltas. Final fresh qualification is pending; no final PASS claim is made.

Total: 0 required findings, 1 optional baseline smell. Same reviewer remains available for focused rechecks.
