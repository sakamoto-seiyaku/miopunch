# MIOPUNCH-49 Standards review

Compared `4185f55caf70bf884a90cbde806d207caf582dba...HEAD` at
`ecb6ea9a9c57128df1c31a694823e49eaa11ba08`: commits `5a780dbc`
(`maintain native membership within node budgets`) and `ecb6ea9a`
(`retain failed recovery pacing and truthful neighbor state`).
Reviewed changed Rust, vendor discovery adapter, runner, tests and decision/evidence documentation.
The follow-up independently reviews the correction delta on top of the previously
reviewed full implementation; it does not adopt the Spec axis's conclusions.

## Documented standards

No hard violations found. Sources: `AGENTS.md`, `/home/js/.codex/RTK.md`,
`CONTEXT.md`, `docs/agents/domain.md`, `docs/agents/issue-tracker.md`,
`docs/agents/triage-labels.md`, relevant ADRs 0001/0003/0004/0005/0007 and
current recovery/path-maintenance notes. No additional non-archive/non-vendor
`CODING_STANDARDS` or `CONTRIBUTING` file was found.

`AGENTS.md` requires current design entry points and explicitly recorded new
decisions; `docs/notes/2026-10-02-node-maintenance.md` records the coordinator,
library reuse, local budget, business lease and persistence choices and their
qualification limits. Archived Go/OpenSpec gates were not treated as current
requirements. Evidence is retained without introducing a competing work ledger,
consistent with `docs/agents/issue-tracker.md`. Formatting and typing are tooling
checks, excluded from this manual axis. User worktree documentation and Cargo
configuration were not edited by this review.

## Heuristic finding, retained for later

**Possible Duplicated Code** — `src/offline_replica.rs:410` adds
`ReplicaProvider::replica_allowed`, duplicating the existing
`NativeNetwork::replica_allowed` at line 258. Both contain:

```rust
(connection.alpn() == REPLICA_ALPN && self.gate.allows(connection, ProtocolScope::Member))
    || (connection.alpn() == crate::network_recovery::RECOVERY_REPLICA_ALPN
        && self.gate.allows(connection, ProtocolScope::Bootstrap)
```

Both also repeat the nonempty/IP-only path predicate. Consider a shared private
predicate taking the gate and Connection when this permission boundary next
changes, keeping the provider's additional current-member check distinct.
The copies currently agree; this is a maintenance heuristic, not a documented
standard violation or demonstrated behavioral bug, and does not block completion.

The correction's two named dialing entry points share one private attempt,
while protocol completion clears failure history explicitly. This small private
boolean does not justify Primitive Obsession or Speculative Generality: its two
behaviors have current callers and recorded purpose. The state correction,
real failing-provider case and narrower runner option add no new Standards finding.

No other baseline smell justified a finding. This review ran no additional tests
and does not independently certify the reported acceptance counts.

Totals: **0 hard violations; 1 nonblocking heuristic**. Spec review remains independent.
