No substantive Standards findings in `e878d95b...34282788 -- tests/admission_gate.rs`.

The correction exercises the agreed public seams: persisted NativeNetwork creation/join, resource configuration, endpoint connection and caller deadlines. Both peers now use the current coordinator protocol; explicit budgets make timeout expectations consistent with the existing setup contract. Assertions still require restricted admission success, business refusal, stalled-attempt closure and a subsequent successful proof. No production mocks, internal test seam, new abstraction or actionable smell was introduced.

Read-only source review; no builds or tests run. The full M92 Standards review remains unchanged.
