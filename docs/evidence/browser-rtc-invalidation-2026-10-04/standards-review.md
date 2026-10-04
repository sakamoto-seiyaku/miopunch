# STEP8 Standards review — initial

Review base: `221950480e92e8b4b29515a74566ccab5693e0a1` (accepted STEP7).
Frozen staged tree supplied for review: `270e7127629430db811abf2ffb3ff69b0d9f8054`.
Diff: `rtk proxy git diff --cached 221950480e92e8b4b29515a74566ccab5693e0a1`.
HEAD confirmed at the base; no unstaged diff or intervening commits. All 56 staged paths are within the authorized STEP8 review scope.

## Findings (under 400 words)

Documented-standard violations: **0**. `AGENTS.md` requires current design entry points and recorded decisions, and excludes archived Go/OpenSpec gates. `docs/agents/domain.md` requires `CONTEXT.md` vocabulary and separation of research/evidence from product decisions. The changed docs preserve these distinctions, explicitly label standard versus diagnostic/component observations, and make no eleven-step completion claim. No independent active coding-standard file was found.

1. **P3 — JUDGMENT: possible Primitive Obsession**, `src/rtc_invalidation_fixture.rs:27,127,146`. The hunk stores `metadata: Value`, then uses `observation.metadata["attempt"].as_str() == Some(attempt)` to decide whether the owned hold may be released. An ownership key is operational state, but its presence and spelling are coupled to a free-form report object populated separately by browser/native callers. Keep the report extensible, but store a typed checkpoint identity (at least an explicit owned attempt field) separately and serialize that identity into diagnostics. This is a maintainability concern, not an observed permission bypass; current callers supply matching identities and the lease/current-M checks still apply.

2. **P3 — JUDGMENT: possible Duplicated Code**, `tests/rtc_invalidation_barrier.rs:19–39` and `tests/rtc_release_validation.rs:21–42`. Both add the same `Gate` and `endpoint` shape: `AcceptData { allowed_data_custom_ids: vec![TRANSPORT_ID] }`, then `Endpoint::builder(presets::Minimal)...clear_relay_transports().add_custom_transport(...).admission_gate(Gate).alpns(...)`. A shared small test helper accepting the distinct ALPN would keep this exact-permission setup from drifting. Preserve the separate tests and their distinct assertions.

Counts: **0 hard violations; 2 judgment findings; worst severity P3 (low), neither an acceptance blocker.**

## Sources and verification boundary

Read current AGENTS/README/CONTEXT/domain guidance, product direction and scenario walkthrough, WebRTC replan and STEP8 note/evidence README, and relevant accepted ADRs 0003/0004/0005/0007. Inspected the full staged source, test and harness diffs, wrapper deltas, and evidence scope/identity fields. The 12 baseline smells were considered; registry/fixture delegation deliberately hides ownership and is not flagged as Middle Man or Feature Envy.

Read-only reviewer: no source/index/build changes, tests, builds, protocol traffic, Plane operations, or diagnostic retries. Recorded PASS claims are reviewed as supplied evidence, not independently rerun. This report alone was written under `/tmp`. Same reviewer remains available for parent fixes/recheck.
