Standards review against `f8f42c1b967f0d0d6df5ebf7fcb187a7f27430e1`: 8 cache/build paths plus 37 necessary createAnswer paths. Applied current AGENTS.md/build-cache rules and the twelve smell heuristics with repository overrides; excluded tooling-enforced formatting and unrelated SDK WIP.

Two original findings, both resolved:

- **P2, cleanup ownership:** `scripts/cargo_cache.py` originally rejected a symlink at the incremental directory but followed symlinked `debug` ancestors, allowing cleanup to delete outside the selected cache. This violated build-cache.md’s preservation rule. The final implementation rejects every descendant symlink ancestor. The meaningful host/triple counterexample produced two failures before the fix; the final six-test run passes and verifies external proof files remain intact.
- **P2, isolation evidence:** `scripts/build_android_pair.py` required an isolation reason but omitted it from the build receipt, contrary to build-cache.md’s “Preserve the reason.” The final report retains `isolationReason`; the existing eight builder tests pass.

Narrow resolution covers only these three changed paths (implementation, regression, receipt metadata). Corrected disk manifest and all 37 immutable capability paths match current bytes. The capability addition requires createAnswer before constructing RTC objects; retained local and actual SDK RED/GREEN evidence supports that limited startup behavior. All 27 compressed originals retain their recorded hashes and exact original bytes.

Existing Native, Android, WASM and selected-test second-build receipts independently report exit zero, zero rebuilds and 527/598/296/529 fresh artifacts. No build or runtime was repeated during review. Cache limits remain explicit pre-start checks; the review does not claim a continuous quota, member communication, physical-device qualification or MIOPUNCH-76 completion.

**Remaining mandatory findings: 0.** No additional smell finding warrants a change in this scope.
