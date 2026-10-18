No actionable Spec findings in `e878d95b...3428278885400d1fa30b89a49fed4ffe0ffdb1e8 -- tests/admission_gate.rs`.

The persisted-network fixture now pairs actual NativeNetwork endpoints through the existing public SDK seam, supplies their required caller deadline, and explicitly disables external resources. Its assertions still require restricted admission-request access, withheld candidates and rejected member business. The stalled-proof fixture selects a short public deadline while retaining rejection, peer closure and successful fresh-connection assertions.

These localized test corrections neither modify production behavior nor rewrite the frozen original-library consumers, and do not expand M92 scope. The prior full source review stands. Supplemental review was read-only; no builds or tests were run.
