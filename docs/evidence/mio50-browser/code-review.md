# MIOPUNCH-50 independent review

Reviewed implementation: `a4f68ddbfc7b4d94aecedc01e054131b21af91fc`; baseline: `03a7653d0495e84e326e0e2cabcd549b4a1a492e`.

Both independent reviews completed before the implementation agent was interrupted. The resumed agent retained their results and the unchanged passing checks; model switching did not require repeating completed work. The reviewers used GPT-6.1 Sol High; their actual runtime service tier was unavailable.

## Standards

Standards review — MIOPUNCH-50

Scope: `03a7653d0495e84e326e0e2cabcd549b4a1a492e...a4f68ddbfc7b4d94aecedc01e054131b21af91fc`; committed changes only.

Hard documented-standard violations: none found. Reviewed AGENTS.md, CLAUDE.md, RTK.md, docs/agents/domain.md, docs/agents/issue-tracker.md, CONTEXT.md, current design notes and relevant ADRs. The platform choices are explicitly recorded in docs/notes/2026-10-02-browser-manual-admission.md; shared native verification and proof modules preserve the current baseline. Evidence is retained as evidence rather than a repository work ledger. No archived Go/OpenSpec gates were applied.

Nonblocking judgement call — possible Primitive Obsession: browser-probe/src/network.rs:123–130 represents the internal admission stage as `fn stage(&self) -> &'static str`, returning `"AwaitingApproval"`, `"PendingMaterials"`, or `"Complete"`. Permission checks then compare `saved.stage() == "Complete"` at line 155 and `self.saved.stage() != "Complete"` at line 453. An internal enum, serialized to these existing public strings at the WASM boundary, would make future stage changes compiler-checked. This is a baseline smell heuristic, not a documented-rule breach or demonstrated bug; park it under the user's instruction rather than require refactoring for completion.

Existing passing validation was inspected, not rerun; unchanged slow NAT checks remain excluded. Total: 0 hard violations, 1 parked maintenance suggestion.

## Spec

No confirmed Spec findings for MIOPUNCH-50.

Reviewed commit `a4f68ddbfc7b4d94aecedc01e054131b21af91fc` against `03a7653d0495e84e326e0e2cabcd549b4a1a492e`, using the complete captured MIOPUNCH-50 and parent MIOPUNCH-14 descriptions, current design notes and relevant ADRs. The reviewed implementation files have no changes relative to that HEAD.

Missing/partial requirements: none identified within this ticket's manual-admission scope. The browser adapter shares native management verification and SCRAM proof code, uses the native manual wire binding, and commits authorization and secret separately through IndexedDB transactions. Web Locks excludes competing pages. Storage clearing, failed commits and incomplete delivery fail closed; close/reopen retains membership without another Add.

Unasked behavior/scope creep: none identified. The SDK, static test page, native operator fixture and isolated harness match the parent's agreed public seams. Precode counting, new-code completion and broader browser control-state synchronization belong to MIOPUNCH-51; their absence is not a MIOPUNCH-50 defect.

Incorrect implementation of requirements: none confirmed. Existing evidence records eleven real browser cases, including admission/data/refresh, freeze/reopen, physical browser process restart, page competition, interrupted delivery, clearing, secret-save failure and unavailable/disabled relay paths. Affected native regressions pass with the documented inert/broker-absent entries excluded from fresh network acceptance. No unchanged slow NAT suite was rerun.

Evidence limits remain explicit: the HTTPS/WSS relay is self-hosted on loopback with a test certificate exception in isolated Chromium and explicit native CA trust. This establishes the browser relay mechanism and manual-admission semantics, without establishing production relay availability or certificate deployment.

Standards: 0 hard violations, 1 parked nonblocking Primitive Obsession suggestion. Spec: 0 confirmed findings.
