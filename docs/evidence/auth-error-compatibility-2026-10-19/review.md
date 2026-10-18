# M96 independent review

Fixed diff: `a6cb8adb19dfe3b8cf95076c6201041dc6ed5163...e240107d3f805a0b18ad1c2979c7e4b0c4ccc922`.

## Standards

Hard documented-standard violations: **0**.

Heuristic findings: **0**.

Reviewed all ten committed paths in `a6cb8adb19dfe3b8cf95076c6201041dc6ed5163...e240107d3f805a0b18ad1c2979c7e4b0c4ccc922` independently of the Spec axis.

The independent `CheckedGroupCrdtError` follows ADR 0012's explicit requirement to preserve the original p2panda error type while expressing new checks separately. `process` and `process_checked` reuse one private processing/causal-validation algorithm; its two actual error types justify the private generic seam. The application conversion preserves original group errors and classifies policy rejection explicitly. The store change updates that existing classification without changing its missing-state branch.

The new compatibility consumer is intentionally immutable and standalone. Its repeated setup is compatibility evidence, not an actionable duplication smell. The new tests exercise causal predecessor state, rejection classification, and durable quarantine/replay through the existing interfaces. No change affects ADR 0010's restricted endpoint separation, ADR 0011's socket identity, dependency locks, build profiles or library defaults.

Read-only source review; no builds, source/Git/Plane mutations, or inspection of the Spec report. Only this review artifact was written.

## Spec

No actionable Spec findings in `a6cb8adb19dfe3b8cf95076c6201041dc6ed5163...e240107d3f805a0b18ad1c2979c7e4b0c4ccc922`.

Reviewed all ten changed paths against M96, parent M91, the accepted ADR baseline and the implementation note. No confirmed missing/partial requirement, unrequested behavior or incorrect implementation was found.

The original six-variant `GroupCrdtError` and ordinary `process` result remain intact. `CheckedGroupCrdtError::Group` preserves the original error, while `PolicyRejected` remains distinct through the explicit root `AuthError` conversion. Both entry points use `process_inner` and the same causal reconstruction/validation. The policy callback still runs on that reconstructed predecessor before `add_operation`; the change introduces no second algorithm or impossible-error panic. Native storage durably quarantines the new policy variant alongside substantive group rejection. Browser merging continues to wait only for its existing explicit missing-history/log cases, so policy rejection does not become pending.

Evidence checked includes the fixed official exhaustive consumer baseline/runtime, unchanged-source fork `E0004`, repaired consumer/runtime and contract-continuity receipt; the permanent consumer hash matches the qualified original bytes. The 21 affected entries include the existing child worker and cover causal/concurrent authorization, missing-dependency backfill, durable rejection, later valid progress and reopen/replay. Final Native and WASM receipts plus all three SDK execution receipts pass with source digest `63c96589d15b52669a2c260e4e274ca03a24eb4fa6635eb2dc19554660323c57`, frozen input/artifact guards and owned-resource cleanup.

The qualified scope is this error-contract change and its affected local SDK flows. It does not complete remaining M91 groups or the independent public/physical-device matrix.

Read-only review; no builds, source/Git edits or Plane mutations. Only this report was written.

Summary: Standards 0 hard / 0 heuristic findings; Spec 0 actionable findings.
