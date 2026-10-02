Standards review — MIOPUNCH-50

Scope: `03a7653d0495e84e326e0e2cabcd549b4a1a492e...a4f68ddbfc7b4d94aecedc01e054131b21af91fc`; committed changes only.

Hard documented-standard violations: none found. Reviewed AGENTS.md, CLAUDE.md, RTK.md, docs/agents/domain.md, docs/agents/issue-tracker.md, CONTEXT.md, current design notes and relevant ADRs. The platform choices are explicitly recorded in docs/notes/2026-10-02-browser-manual-admission.md; shared native verification and proof modules preserve the current baseline. Evidence is retained as evidence rather than a repository work ledger. No archived Go/OpenSpec gates were applied.

Nonblocking judgement call — possible Primitive Obsession: browser-probe/src/network.rs:123–130 represents the internal admission stage as `fn stage(&self) -> &'static str`, returning `"AwaitingApproval"`, `"PendingMaterials"`, or `"Complete"`. Permission checks then compare `saved.stage() == "Complete"` at line 155 and `self.saved.stage() != "Complete"` at line 453. An internal enum, serialized to these existing public strings at the WASM boundary, would make future stage changes compiler-checked. This is a baseline smell heuristic, not a documented-rule breach or demonstrated bug; park it under the user's instruction rather than require refactoring for completion.

Existing passing validation was inspected, not rerun; unchanged slow NAT checks remain excluded. Total: 0 hard violations, 1 parked maintenance suggestion.
