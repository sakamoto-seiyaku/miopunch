# Standards review — MIOPUNCH-51

Reviewed committed diff `f997db370cc1451641afd1f04f2b41286ec8301b...f50b648c3fd791e1c3827f522bc14325cc42223c` using the code-review skill’s Standards axis.

**Documented-standard violations: none found.** Sources: `AGENTS.md`, `/home/js/.codex/RTK.md`, `CONTEXT.md`, `docs/agents/domain.md`, `docs/agents/issue-tracker.md`, current design entry points and relevant ADRs. The diff retains the shared authorization verifier and explicit source-scoped completion vocabulary, documents the dependency adaptation and platform limits, and does not apply archived workflows or introduce a repository work ledger. Tooling-enforced matters are excluded.

**Heuristic findings (optional, nonblocking):**

1. **Possible Duplicated Code — replica resource limits.** `browser-probe/src/replica.rs:13` and `src/replica_snapshot.rs:16` both introduce `const MAX_RECORDS: usize = 4096;` and `const MAX_BYTES: usize = 16 * 1024 * 1024;`, matching the remaining native constants at `src/offline_replica.rs:25`. These constrain the same exchanged snapshot; future limit changes can leave sender and receiver disagreeing. Consider exporting the common limits from the shared snapshot module when this area next changes.
2. **Possible Duplicated Code — precode ledger count.** `browser-probe/src/admission.rs:41` uses `.filter(|a| a.code == Some(code.token) && a.authorization.is_some())`; line 286 repeats `.filter(|a| a.code == Some(token) && a.authorization.is_some())`. Restore verification and new-redemption preflight independently encode the same definition of a consumed use. A small `SavedNode` helper could keep that predicate aligned.

Neither finding establishes a behavioral bug or requires an architecture refactor before completion. Unmodified upstream vendor Rust was excluded; the declared dependency patch/provenance was inspected. No runtime files changed and no tests rerun; this report makes no claim about the ongoing final regression result.

Summary: 0 hard violations; 2 low-priority heuristic findings, both duplication risks.
