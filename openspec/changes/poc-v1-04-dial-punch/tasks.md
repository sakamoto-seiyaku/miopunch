## 1. Punch Objects + Handoffs

- [ ] 1.1 Add `internal/pocv1/punch` and implement `dial_offer` / `dial_answer` with the fixed v1 body fields.
- [ ] 1.2 Define `PathResult` as this change's only output: selected UDP path, ownership, and punch evidence.
- [ ] 1.3 Keep `member_credential` handoff explicit for `05`; do not mix recipe selection into `PathResult`.

## 2. UDP-only Attempt Runtime

- [ ] 2.1 Implement inbox-topic candidate exchange using `01` peer-targeted wire/security plus `06` `roster_snapshot + TopicScope`.
- [ ] 2.2 Implement the 5B attempt matrix with max concurrency 4 and total budget 10s.
- [ ] 2.3 Reuse legacy punching/connectivity only as leaf mechanics behind the new `internal/pocv1/punch` orchestrator.

## 3. Acceptance

- [ ] 3.1 Add tests for `dial_offer/dial_answer` roundtrip, bounded attempt scheduling, timeout aggregation, and selected-path evidence.
- [ ] 3.2 Add a focused two-peer smoke proving `PathResult` can be produced from roster-backed peer identity and inbox derivation without invoking session upgrade code.
