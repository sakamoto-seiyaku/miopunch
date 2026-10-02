# MIOPUNCH-52 Standards review

Reviewed commit `310a6f841ecadb984e6caaf17313594355b6445d` against base `17b508576bebb8a382761641a99b02a40e4c64b3`. Read current AGENTS/domain guidance, CONTEXT, design entry points, accepted ADRs, delivery note, changed application code, runners, packaging changes and evidence index. Protected unrelated working-tree changes are excluded. No tests or tooling-enforced checks were rerun.

## Hard documented-standard violations

None found. `AGENTS.md` requires explicit recording of new decisions and excludes archived Go/OpenSpec rules. The delivery note records protocol selection, Windows persistence, packaging rationale and compatibility limits. `docs/agents/domain.md` requires current domain vocabulary and surfacing ADR conflicts; no conflict was found in the reviewed changes. The shell delegates authority, admission and synchronization to existing core operations. No separate current coding-style standard was available.

## Optional baseline smells — parked

- `src/precode.rs:133` — **Duplicated Code** (“same logic shape in multiple hunks/files”). The new usage-status query repeats the ledger-consistency predicate already used at lines 194 and 289–294: `used=(SELECT count(*) FROM precode_redemptions ...)` plus the missing/incomplete-delivery check. A future change to what constitutes trustworthy consumption must update all three. Consider one named SQL predicate or an equivalent shared database abstraction when that policy next changes; preserve the atomic redemption guard. This is a maintenance judgment, not a demonstrated bug or documented-standard violation.
- `scripts/check_desktop_management.py:99`, `:171`, `:270` — **Duplicated Code** (“same logic shape in multiple hunks/files”). The restart, precode and first-join scenarios repeat creator/applicant setup and identity/session opening. A small fixture helper could reduce drift, but explicit scenario steps aid evidence review; no refactor is required for delivery.

No other actionable baseline smell identified. Unchanged vendored Rust is treated as upstream import, not an invitation to apply project refactors. Optional observations remain parked under the user’s completion priority.
