# MIOPUNCH-60 final generation 3 Standards review

Reviewed the authorized WIP against `91b37a50fd509dd47c221059718406feafa8ab3a`, including the formal connector/direct module, integration diff, vendor endpoint/socket seam and patch notes, assisted regression, harness/verifiers, validation notes, and retained/compressed evidence. Unrelated original dirty documentation and historical ADR work were excluded.

Required corrections: **None found.** No actionable breach of AGENTS.md/RTK, current domain-document rules, or accepted ADR boundaries was identified. The native-only assisted seam stays near the socket/interface data it owns, attaches actual local addresses in both observation paths, and documents the vendor extension in `vendor/iroh/UPSTREAM.md:131`. Existing identity, authorization, original-Connection, and resource boundaries remain intact. The updated documentation explicitly identifies the assisted omission in historical generations and does not claim their 14/20 result as a generation 3 success rate.

Optional suggestions — baseline heuristics, not documented-standard violations:

- **Possible Mysterious Name:** `src/bin/udp_nat_probe.rs:273` — `automatic_strategy(_has_mapping_observers)` selects only the experimental `single-candidate` branch, returning false for `auto`/`formal`. A future rename and removal of the unused observer argument would describe its current purpose.
- **Possible Duplicated Code:** `src/nat_connection.rs:106`, `:306`, `:473` — captured-scope checks and 10 ms polling loops repeat. A small shared scope-check helper could reduce omission risk, while preserving commit cancellation/ownership and explicit checks before business IO.

Neither optional suggestion requires broadening the frozen implementation.

Validation: read-only independent verifier calls passed all **40 historical comparison cells, three historical functional chains, and two generation 3 assisted checks**. All **49** generation 3 frozen source hashes match current files. Independently checked all **191** compressed streams against compressed/original lengths and SHA256 round-trip receipts. Both fault-receipt hashes match their canonical strict artifacts. Inspected actual RED/GREEN records and the final real-MQTT suite log: **241 passed, 0 failed, 4 existing ignored**, with recorded exit 0. No runtime/probability trials, source/Plane/index changes, or canonical evidence writes were performed.

Standards findings: **0 required, 2 optional heuristics**.

Packaging addendum: reviewed the new supplemental fault script and README command. Corruptions affect temporary copies; only the fault receipt is published. Stream 192 preserves the GREEN log's exact recorded bytes, length, and hashes. All 49 frozen inputs remain unchanged. **No required findings added; no runtime or fault trials rerun.**
