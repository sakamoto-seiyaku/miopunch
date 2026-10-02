# MIOPUNCH-49 two-axis review

Fixed point: `4185f55caf70bf884a90cbde806d207caf582dba`. Final source: `ecb6ea9a9c57128df1c31a694823e49eaa11ba08`. Independent GPT-6.1 Sol HIGH Fast/priority agents reviewed both commits.

## Standards

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

## Spec

# MIOPUNCH-49 independent final Spec review

Reviewed the complete comparison
`4185f55caf70bf884a90cbde806d207caf582dba...ecb6ea9a9c57128df1c31a694823e49eaa11ba08`
(commits `5a780dbc` and `ecb6ea9a`) against actual MIOPUNCH-49 and parent
MIOPUNCH-14, the current design baseline, recorded integration boundaries, and
the final/post-review evidence. Spec axis only. **Zero confirmed unresolved
specification findings; no confirmed scope creep.**

The [initial review](spec-review-initial.md) identified two P2 deviations. Both
are resolved in the reviewed implementation:

1. MIOPUNCH-49 requires “连续失败后退避” (line 9 of the saved Plane description).
   Maintenance now uses `ResourceEndpoint::connect_for_recovery`, preserving
   accumulated failures across successful QUIC handshakes. The coordinator
   resets pacing after complete, authorized, durable recovery; incomplete or
   failed recovery still records failure. The actual authenticated-peer regression
   first succeeds at LogSync, then sends an invalid manifest after real TLS/QUIC.
   [RED](review-failure-red/results.json) observed handshake intervals
   1739/1396/1395 ms; [GREEN](review-failure-green/results.json) observed
   1720/2407 ms, confirming progressive failure pacing rather than handshake
   success erasing recovery failures.

2. MIOPUNCH-49 requires “所有入口失败时持续可解释等待/退避” (line 18);
   parent MIOPUNCH-14 requires “得到暂不可达状态” (line 152).
   Existing-neighbor sync failure now sets `TemporarilyUnreachable`, and an empty
   neighbor view no longer relies on historical `last_sync` being absent before
   reporting waiting. The publishing fallback also corrects stale `Connected`.
   Historical source/scope completion remains available. The same real regression
   observed `stale_connected=true` before the fix and `false` afterward.

[Post-review verification](post-review/results.json) passes all twelve ordered
acceptance cases, 26 resource checks, 12 recovery checks, all-target typechecking
and formatting. Its repeated failure check independently records 1699/2379 ms
and `stale_connected=false`. Earlier unaffected short/library/WASM evidence
remains in [final-suite](final-suite/results.json).

These results qualify the recorded native Linux controlled environment and
supported carrier profile. No source edits, new tests, or unchanged slow NAT
reruns were performed by this reviewer.

Totals: Standards 0 hard violations / 1 parked nonblocking heuristic; Spec 0 confirmed unresolved after 2 initial findings were fixed.
