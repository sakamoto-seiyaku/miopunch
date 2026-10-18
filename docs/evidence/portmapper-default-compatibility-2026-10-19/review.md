## Standards

0 findings. See review-standards.md.

## Spec

0 findings. See review-spec.md.

Both reviewers inspected the implementation and existing receipts without rerunning builds or runtime checks.

No Standards findings in 1ddafa9d…49213d3a.

Documented violations: none. Enabled matches upstream provenance and ADR 0012. Product/fixture callers explicitly choose Disabled. Saved resource consent and mapping ownership rules remain unchanged. The restricted delegate matches the ordinary feature gate.

Baseline smells: none warrant reporting. Repeated explicit Disabled calls state caller policy; centralizing this in the generic wrapper would undermine the required library default.

Reviewed all 50 changed files, build guards, full selector lock rows, consumer continuity, ordinary/feature-off graphs, original/restored runtime receipts, fixture typecheck, artifact requalification and reviewed cache cleanup.

Limits: read-only review; no compilation/runtime reruns, edits or tracker mutations. Evidence packaging outside review scope.

No Spec findings.

MIOPUNCH-98 restores ordinary Enabled behavior without forcing Cargo features. Product resource assembly preserves saved Disabled/NatPmp choices; probes and fixtures declare their intent explicitly. The restricted delegate matches the ordinary method feature gate.

Reviewed evidence covers the unchanged pre-edit consumer, real failures, restored gateway mapping and PING/PONG, feature-off silence, 12 product gateway cases, owner isolation, cancellation/timeout release, CLI packet silence, and Native/Browser SDK durability and natural cleanup. Close returning and observed gateway deletion are separate.

Limits: read-only source/controller/receipt review; no builds or runtimes rerun. Controlled facilities only; arbitrary-router cleanup guarantees and broader physical/public matrix excluded.
