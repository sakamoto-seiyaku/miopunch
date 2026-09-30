# MIOPUNCH-60 final Standards review

Reviewed the authorized working-tree diff against `91b37a50fd509dd47c221059718406feafa8ab3a`, including the listed untracked connector, direct-path module, harness, verifier, validation notes, and evidence. HEAD itself remains the baseline; this review did not use its empty three-dot diff.

Required corrections: **None found.** No actionable breach of AGENTS.md/RTK, the current domain-document rules, or accepted ADR boundaries was identified. The September 30 NAT migration decision explicitly authorizes the limited legacy behavior/fixture reuse; the change does not restore the archived general workflow. Evidence clearly separates probability samples, functional chains, direct successes, misses, and actual runtime generations.

Optional suggestions (code-review baseline heuristics, not documented-standard violations):

- **Possible Mysterious Name** — `src/bin/udp_nat_probe.rs:273`: `automatic_strategy(_has_mapping_observers)` now returns false for `auto`/`formal` and true only for `single-candidate`. Rename it to identify that experimental branch and remove the unused observer argument; update the associated `automatic` locals. The present name describes the former behavior and can confuse future routing changes.
- **Possible Duplicated Code** — `src/nat_connection.rs:105`, `:306`, `:465`: captured generation, authorization scope, connection closure, and permission checks recur with similar 10 ms polling loops. A small shared scope-check helper could reduce future omission risk. Preserve the different commit cancellation/ownership behavior and the explicit business check before polling IO; this is optional follow-up, not a request to broaden this frozen implementation.

Validation: independently invoked the verifier's read-only `verify()` for all 40 comparison cells and three functional chains: **43 PASS**. Generation 2's 47 frozen source hashes match current files; generation 1 differs only in the two documented corrections. The strict receipt matches the 16-fault receipt hash. Inspected the real-MQTT suite log and accounting/provenance receipts. No new runtime/probability trials, source/tracker/index mutations, or canonical evidence writes were performed.

Standards findings: **0 required, 2 optional heuristics**.
