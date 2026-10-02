# MIOPUNCH-51 two-axis code review

Implementation reviewed: `f50b648c3fd791e1c3827f522bc14325cc42223c`; fixed baseline `f997db370cc1451641afd1f04f2b41286ec8301b`. Independent reviewers used GPT-6 Astra Low; root read back actual priority service tier for both.

## Standards

Reviewed committed diff `f997db370cc1451641afd1f04f2b41286ec8301b...f50b648c3fd791e1c3827f522bc14325cc42223c` using the code-review skill’s Standards axis.

**Documented-standard violations: none found.** Sources: `AGENTS.md`, `/home/js/.codex/RTK.md`, `CONTEXT.md`, `docs/agents/domain.md`, `docs/agents/issue-tracker.md`, current design entry points and relevant ADRs. The diff retains the shared authorization verifier and explicit source-scoped completion vocabulary, documents the dependency adaptation and platform limits, and does not apply archived workflows or introduce a repository work ledger. Tooling-enforced matters are excluded.

**Heuristic findings (optional, nonblocking):**

1. **Possible Duplicated Code — replica resource limits.** `browser-probe/src/replica.rs:13` and `src/replica_snapshot.rs:16` both introduce `const MAX_RECORDS: usize = 4096;` and `const MAX_BYTES: usize = 16 * 1024 * 1024;`, matching the remaining native constants at `src/offline_replica.rs:25`. These constrain the same exchanged snapshot; future limit changes can leave sender and receiver disagreeing. Consider exporting the common limits from the shared snapshot module when this area next changes.
2. **Possible Duplicated Code — precode ledger count.** `browser-probe/src/admission.rs:41` uses `.filter(|a| a.code == Some(code.token) && a.authorization.is_some())`; line 286 repeats `.filter(|a| a.code == Some(token) && a.authorization.is_some())`. Restore verification and new-redemption preflight independently encode the same definition of a consumed use. A small `SavedNode` helper could keep that predicate aligned.

Neither finding establishes a behavioral bug or requires an architecture refactor before completion. Unmodified upstream vendor Rust was excluded; the declared dependency patch/provenance was inspected. No runtime files changed and no tests rerun; this report makes no claim about the ongoing final regression result.

Summary: 0 hard violations; 2 low-priority heuristic findings, both duplication risks.

## Spec

Reviewed committed range `f997db370cc1451641afd1f04f2b41286ec8301b...f50b648c3fd791e1c3827f522bc14325cc42223c` against the live MIOPUNCH-51 description, cached parent MIOPUNCH-14, current context and relevant accepted ADRs. Inspected implementation, harness assertions and saved evidence; no tests rerun by this reviewer.

**Findings: 0.** No concrete missing/partial requirement, unasked scope expansion or incorrect implementation found within this ticket’s browser-control scope.

- “浏览器持久保存另一作者记录，重载后在原作者离线时仍能向第三节点提供完整副本。” The replica case stops the original native author, reloads the browser, serves a third browser, checks the original body and reloads that receiver. Shared LogSync snapshots preserve original signed records.
- “缺正文、重复、乱序和存储失败不造成错误同步完成或越权视图。” Complete-record validation, durable pending records and commit-before-gate updates match the fault assertions: missing body rejected, missing dependencies incomplete, pending surviving reload, duplicate/reversed input applied once, and actual IndexedDB transaction abort preventing completion.
- “日常操作和创世操作的权限矩阵与原生一致，管理员退出和撤销后的连接处理一致。” Shared action classification and causal verifier enforce the matrix; browser session expiry is checked again before signing. Tracked real connections close on committed removal, and new connections are rejected. Administrator leave routes through the protected remove action.
- “清理站点数据或恢复失败时行为明确” and “明确页面分发信任和敏感材料生命周期。” Existing clear-data checks and new corrupt-state checks fail explicitly; the implementation note documents origin trust, page-local root elevation and private durable precode state.
- “浏览器 precode 核销、跨页面同次重试及原交付者不可用后的新码补交沿共享实现运行，不重复扣原码或重复添加有效成员。” Shared request namespaces, redemption policy and authorization verification support the atomic browser ledger. Assertions cover page ownership transfer, issuer reload, one-use retries, old-code invalidation after demotion/reappointment, and another browser administrator completing with a new code after the original issuer stops.

The dependency feature patch and native receipt acknowledgement support this delivery; deferred UI, migration, rotation and NAT work remain outside it. Saved final-suite results show all 19 commands exiting successfully. This review does not imply broader browser/platform qualification or independent execution of those checks.

Standards: 0 hard violations, 2 optional duplication heuristics; Spec: 0 findings. The optional maintenance suggestions remain parked under the user's completion priority; no confirmed bug remains.
