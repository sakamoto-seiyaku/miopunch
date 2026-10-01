# MIOPUNCH-40 standards review

Reviewed snapshot `0b9df2cc62b9b22fe4b34e10c5b895a067274786` against
`db292f540d382a9a8e7ae408f2d657afa8fa63f7` using the three-dot diff.
Scope: runtime, fork adaptations, formal TCP tests, runner, and accompanying
validation/evidence documentation.

**Documented-standard breaches: none found.**

Standards consulted: `AGENTS.md`, `/home/js/.codex/RTK.md`,
`docs/agents/{domain,issue-tracker,triage-labels}.md`, `CONTEXT.md`, current
product/scenario and transport notes, and ADRs 0001, 0004 and 0005. Archived
Go/OpenSpec rules were excluded as required by `AGENTS.md`.

The change preserves the documented distinctions between node identity,
membership, direct peer paths and auxiliary resources. New configuration and
wire/lifecycle choices are explicitly recorded in the TCP validation note;
research suggestions are not presented as product decisions. The evidence
index retains acceptance evidence without becoming an open-work ledger, as
required by `docs/agents/issue-tracker.md`.

**Optional baseline smell findings: none warranting a change.**

The shared `ResourceConfig::allows_custom` policy centralizes qualification.
Checks at candidate selection and actual send serve different enforcement
boundaries. The fork wrapper and public adapter helpers provide necessary
policy and encoding seams; their delegation does not warrant a Middle Man
finding. IPv4 compatibility helpers preserve existing callers.

This was an independent static standards review, not a new runtime validation
run. Tooling-enforced formatting/typechecking and unchanged slow NAT matrices
were not rerun or imposed as additional gates.

Total: **0 required violations; 0 optional smell findings.**
