## Standards

0 documented violations; 0 actionable heuristic findings.

## Spec

0 findings. All four acceptance criteria supported by inspected implementation and existing runtime evidence.

Two independent read-only reviews of 802742264d0f26ba96ec8af41b35f5a0ab3b7829...fea33a646a60f67218d50892f773ac9c81d405f9. Neither reviewer reran builds or runtime checks.

Findings: Standards 0 (none); Spec 0 (none).

## Standards report

# MIOPUNCH-100 Standards review

Reviewed commit `fea33a646a60f67218d50892f773ac9c81d405f9` against
`802742264d0f26ba96ec8af41b35f5a0ab3b7829`: one commit, all ten changed paths.

**Documented violations: none.** Ordinary missing-transport errors and explicit
resolving methods follow ADR 0012. Ordinary and restricted wrappers share the
existing connection path, consistent with ADR 0010; no socket ownership change
conflicts with ADR 0011. Public backoff documentation states its sampling bounds,
and validation prevents invalid sampling and overflow while retaining the clamp.

**Baseline smells: none warrant reporting.** The shared `connect_inner` and
combined actor arm centralize connection behavior. Restricted-wrapper delegation
maintains the required interface boundary. Explicit resolving is a present
requirement, and its private binary choice does not justify another strategy
abstraction. Regression tests exercise public behavior, real streams, error
classification and sampling boundaries.

Sampled evidence includes the fixed original consumer before production edits,
original/red/final continuity, the shared-cache build driver with source, lock,
artifact and tool guards, complete foreign lock-row comparisons, final regression
and root typecheck receipts, wrapped CLI and restricted-wrapper runtime exits,
runtime selected-input freezing, lossless intermediate-test compression,
historical deduplication preserving paths and bytes, and the commit receipt's
249 protected WIP hashes and dirty-set guard. These are consistent with
`AGENTS.md` and `docs/agents/build-cache.md`.

Limits: independent read-only inspection; no compilation, tests, Git/Plane
mutations or source edits. Only this requested report was written. Evidence
packaging and the broader physical/public matrix were outside this review.

## Spec report

No Spec findings.

Reviewed the ten committed paths in `802742264d0f26ba96ec8af41b35f5a0ab3b7829...fea33a646a60f67218d50892f773ac9c81d405f9` against MIOPUNCH-100, the relevant M91 requirements, CONTEXT and ADRs 0010–0012. No missing requirement, scope creep or incorrect implementation was identified.

Ordinary calls restore the existing immediate `TransportInfoMissing` result; address-book failures return before lookup. Both explicit resolving methods share the original connection actors and are adopted by the root wrapped probe. Existing local-rejection classification remains intact. Backoff validates its documented whole-millisecond increment bounds and saturates before the existing ceiling clamp.

Existing evidence supports all four acceptance criteria: unchanged original/RED/final consumer bytes; eight actual public regression cases including real streams and an active reachability-reporting guard; restricted-wrapper admission/revocation; three CLI cases; root typecheck; and the current Native/Browser SDK approval, durable materials, identity reopen, member PONG and natural cleanup.

Limits: this review inspected source, controllers, raw targeted outputs and receipts without rerunning builds or runtimes. The original baseline isolates official p2panda source using the pinned fork dependency graph; it does not certify an entirely upstream dependency stack. Resolving retains existing actor cancellation behavior, and the broader public/physical matrix remains separate. Pending evidence packaging was not treated as an implementation defect.
