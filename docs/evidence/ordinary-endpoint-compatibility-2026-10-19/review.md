# M95 independent review

Fixed diff: `ba5f544a55536789b17278646785a4d0fde01a46...d4789c6ae6c9434c448ffd24575baba44b119413`.

## Standards

Hard documented-standard violations: **0**.

Heuristic findings: **0**.

Reviewed all 13 committed paths in `ba5f544a55536789b17278646785a4d0fde01a46...d4789c6ae6c9434c448ffd24575baba44b119413` independently of the Spec axis.

The production changes remove transitional admission configuration and hook branches, restore the ordinary incoming early-data return type and exhaustive hook outcomes, and retain the independent restricted admission pipeline. This follows ADRs 0010/0012; the socket-identity interfaces governed by ADR 0011 are preserved. The native-only development dependency changes only the local package lock dependency row, consistent with the pinned-graph rule in `docs/agents/build-cache.md`. No new structured events implicate `vendor/iroh/DEVELOPMENT.md`.

The standalone original consumers intentionally preserve their qualified source bytes. Their self-contained setup and required socket delegation are compatibility evidence, so no duplication, Middle Man, or formatting finding is warranted. The Cargo wrapper registers the same programs without introducing an alternative implementation.

Read-only review: no builds, source/Git/Plane mutations, or inspection of the Spec report. Only this review artifact was written.

## Spec

No actionable Spec findings in `ba5f544a55536789b17278646785a4d0fde01a46...d4789c6ae6c9434c448ffd24575baba44b119413`.

Reviewed all 13 committed paths against M95, parent M91, ADRs 0010/0011/0012 and the implementation note. No confirmed missing/partial requirement, unrequested behavior or incorrectly implemented requirement was found.

Ordinary incoming early-data again returns its original direct type; outgoing early-data retains `Result`. The restored hooks file matches the pinned official source, including exhaustive Accept/Reject and hook order/short-circuiting. The unused hook-based admission builder/configuration and expanded outcomes are removed. The optional independent admission policy consistently controls TLS early-data suppression, restricted registration and sole-custom-route ownership. Restricted wrappers still provide no ordinary early-data/conversion path, and final policy/path checks and legacy-hook veto remain before business handoff. The residual-consumer inventory found no Rust consumers of the removed hooks/outcomes/configuration.

Evidence checked: the before-production baseline and same-source fork failure; unchanged official/fork early-data and routing consumers, with committed consumer hashes matching the fixed programs; the original M92 consumer; 18 compiler contracts; 36 distinct affected runtime tests, including the explicitly enabled historical counterpart case; and final Native/WASM build plus four SDK execution receipts. The final receipts identify source digest `f48dcdbcf04cf118184ae076f2e3d9289d369ac81dd0681bb5ef02f376f1f231`, passing input/artifact guards and natural cleanup. Historical controller labels do not override those explicit subjects.

These results support M95's ordinary-contract restoration and affected restricted/SDK flows. They do not claim completion of the broader M91 compatibility work or public/physical-device matrix.

Read-only review; no builds, source/Git edits or Plane mutations. Only this report was written.

Summary: Standards 0 hard / 0 heuristic findings; Spec 0 actionable findings.
