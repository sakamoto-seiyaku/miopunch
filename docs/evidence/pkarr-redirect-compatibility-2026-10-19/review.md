## Standards

0 findings. See review-standards.md.

## Spec

0 findings. See review-spec.md.

Both reviewers inspected code and existing evidence without rerunning builds or runtime checks.

No Standards findings in 67d4f496…12623770 (one commit, seven changed files).

Documented violations: none. Ordinary constructors recover upstream redirect defaults under ADR 0012; constraints require explicit policy. Existing public signatures and trait implementations remain usable. Documentation distinguishes ordinary behavior, constrained behavior and third-party declarations.

Baseline smells: none warrant reporting. Constructor logic and endpoint/caller policy intersection are centralized. Wrapper delegation preserves the new capability contract. Publisher metadata reflects constraints enforced by its separately owned background client.

Evidence sampled: unchanged-consumer continuity, selected product-test provenance, shared-cache driver guards, complete foreign lock-row validation, final ordinary/SDK builds, successful fixture typecheck, Browser cleanup, runtime input freezing and reviewed cache pruning. Failed preparation receipts remain separately identifiable.

Limits: read-only review of every changed hunk; no builds, tests, edits or tracker writes. Evidence packaging was outside this review.

No Spec findings.

The change restores ordinary native redirects and adds explicit constraints through client, publisher and resolver builders. Endpoint policies are intersected with caller restrictions; constrained native requests disable redirects and recheck permission. Browser constrained requests stop before Fetch. Conservative registry checks and wrapper propagation prevent ordinary prebuilt services from bypassing product restrictions.

Reviewed evidence includes unchanged published/fork consumers, real RED failures, successful final signed PUT/GET, nine resource tests, Chromium cross-origin behavior, and the full Native/Browser SDK story with natural cleanup.

Limits: inspected existing source/controllers/receipts without rerunning checks. The inaccessible original client builder is covered by a clearly labelled private-field test adapter. External service declarations remain contracts, without sandboxing independently started actors; product policies remain startup snapshots.
