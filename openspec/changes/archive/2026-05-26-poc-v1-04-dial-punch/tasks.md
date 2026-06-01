## 1. Punch Objects + Handoffs

- [x] 1.1 Add `internal/pocv1/punch` and implement `dial_offer` / `dial_answer` with the fixed v1 body fields.
- [x] 1.2 Define `PathResult` as this change's only output: selected UDP path, ownership, and punch evidence.
- [x] 1.3 Keep `member_credential` in `dial_offer` / `dial_answer`, but validate it against inner sender, trusted roster, and authority signature before folding the trusted identity into `PathResult`.

## 2. UDP-only Attempt Runtime

- [x] 2.1 Implement inbox-topic candidate exchange using `01` peer-targeted wire/security plus `06` `roster_snapshot + TopicScope`.
- [x] 2.2 Implement the 5B attempt matrix with max concurrency 4 and total budget 10s.
- [x] 2.3 Reuse legacy punching/connectivity only as leaf mechanics behind the new `internal/pocv1/punch` orchestrator.

## 3. Acceptance

- [x] 3.1 Add tests for `dial_offer/dial_answer` roundtrip, bounded attempt scheduling, timeout aggregation, and selected-path evidence.
- [x] 3.2 Add a focused two-peer smoke proving `PathResult` can be produced from roster-backed peer identity and inbox derivation without invoking session upgrade code or a second roster lookup in `05`.
- [x] 3.3 Add `verifyRemoteAssertion` / `resolveTarget` / `trustedRemoteFromRoster` failure-path tests for sender Ed25519 mismatch, malformed credential, authority verify failure, invalid target peer ID, offline target, missing roster entry, and roster peer ID mismatch.
- [x] 3.4 Add `verifyOffer` / `verifyAnswer` / `exchangeOffer` / `waitAndAnswerOffer` exchange-boundary tests for wrong inner kind, malformed body, mismatched `dial_id`, mismatched `punch_token`, mismatched `in_reply_to`, non-target answer peer, ignored unrelated answers, and ignored invalid offers before a valid one.
- [x] 3.5 Add `normalizeCandidates` / `buildPairPlans` / `withAttemptBudget` / `executePairPlans` / `PathResult.Close` runtime and config tests for trim-dedupe-sort behavior, invalid candidate inputs, no candidate pairs, budget timeout cause, `failed` evidence mapping, winner-driven cancellation, and nil-conn close.
- [x] 3.6 Re-run focused 04 acceptance with `go test ./internal/pocv1/punch/...`.
