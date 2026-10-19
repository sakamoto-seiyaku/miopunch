# Final two-axis review — MIOPUNCH-102

## Standards

0 documented-standard violations; 0 actionable heuristic findings. The unknown-to-unknown binding edge raised during review is resolved by the minimal reset condition and public-input, actual encrypted PING/ACK-carrier regression.

## Spec

0 remaining findings. The original P2 (recoverable/passive local-IP changes retaining preference) and repeated None→None edge are resolved. Invalid passive server-fixture failure remains identified as preparation evidence, not a valid product RED. The final 51 outer tests and full rebuilt Native/Browser SDK flow qualify at the frozen final source digest.

Reviewers inspected source and existing receipts; they did not rerun tests. Final archive packaging is separately verified by its complete member readback and private-material scan. Broader public/physical matrix remains independently scoped.

Findings by axis: Standards 0; Spec 0. No outstanding severity.

## Standards report

# MIOPUNCH-102 final Standards review

Baseline: `3be145a09bf8847d767e101a9b8c4a8642bf1070`.
Reviewed the seven vendor paths, final contract note and three frozen consumers.
Final sampled source digest:
`6d5e2bcdcb76f911e31dc6e02bfcb216bae278d6ed8bdfba011a274194307a54`.

**Documented-standard violations: none outstanding.** Ordinary ACK routing and
explicit probe selection follow ADR 0012. Binding-scoped reset remains with
`PathData`, consistent with ADR 0011; packet/frame permission boundaries remain
unchanged under ADR 0010. The previous unknown-to-unknown reset edge is closed:
`set_local_ip` now clears whenever the new IP is unknown or differs, while a
matching known IP preserves selection. Its regression uses public recovery
notifications and an authenticated packet to observe the actual ACK carrier.

**Heuristic smells: none warrant an actionable finding.** Reset logic is
centralized at the binding owner; ACK tests share their carrier assertions.
No extra production configuration or speculative interface was introduced.
Duplication between frozen evidence consumers preserves their qualified bytes
and is justified by the compatibility requirement.

Read-only evidence sampling found final qualified receipts for 17 protocol,
12 socket, 15 unified and seven maintenance tests: 51 outer tests, zero failures.
The owned broker exits zero and both containers are absent. The Browser build
is qualified; SDK execution is qualified at the same source digest. Runtime
freezing records all 604 sources and five final graphs without selected-input
changes. Build drivers retain complete foreign lock rows, pre-Cargo source,
lock, artifact and tool guards, shared target ownership and unchanged profiles,
consistent with `AGENTS.md` and `docs/agents/build-cache.md`. Historical test
compression records byte-exact roundtrips; SDK binaries remain excluded.

Limits: no tests, builds, cleanup, source edits or Git/Plane mutations were
performed. Only this requested report was written; private SDK materials were
not copied. Final archive packaging and broader public/physical acceptance
remain outside this review.

## Spec report

No remaining Spec findings.

Reviewed the owned MIOPUNCH-102 diff against `3be145a09bf8847d767e101a9b8c4a8642bf1070`, its five acceptance criteria, the selected contract and ADR 0012. Protected WIP was excluded. The final implementation and inspected evidence agree with the selected explicit-probe classification; no missing behavior, scope creep or incorrect implementation remains identified.

Repeated unchanged ordinary consumers establish the default-routing regression and its restoration, including single/multipath streams, delayed backup returns, actual packet loss, recovery and close observations. Existing probe entry points explicitly select only the current binding. Authenticated watermarks, packet/frame permissions and close routing remain intact; gate-dropped STREAM/PING packets receive no ACK, while permitted replay delivers the original data.

The earlier P2 is resolved: all three in-place local-IP update sites use `set_local_ip`; changed or cleared addresses reset the preference. The supplemental `None → None` edge is also resolved: unknown bindings always clear it, while unchanged known bindings retain it. The actual encrypted client-PING regression fails before that fix and passes afterward. The invalid original server-direction passive-migration failure is correctly recorded as fixture preparation, not product RED.

Final evidence includes 51 passing outer tests (17 protocol, 12 socket, 15 unified, seven maintenance), repeated ordinary/explicit consumers, real injected-loss recovery, and the full current Native/Browser SDK approval, durable materials, identity reopen, member PONG and natural cleanup. Five builds share final source digest `6d5e2bcdcb76f911e31dc6e02bfcb216bae278d6ed8bdfba011a274194307a54`; all 604 current source hashes match the runtime guard, and the three durable consumer copies match their frozen inputs byte-for-byte.

Limits: inspected source and existing raw evidence/controllers/receipts without rerunning checks. The documented stale-Available close boundary and separate public/physical matrix remain. Final archive packaging awaits its separate integrity review.
