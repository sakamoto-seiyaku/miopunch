## 1. Persist Foundation

- [x] 1.1 Add `internal/pocv1/persist` and implement the caller-supplied root plus `device/` + `networks/<canonical network_id>/` layout resolver.
- [x] 1.2 Implement the first typed authority only for device keys, `self_member_credential`, `mailbox_secret`, single runtime broker config, and whole `roster_snapshot`.
- [x] 1.3 Implement `TopicScope` derivation for `net_root`, `presence_topic(peer_id)`, and `inbox_topic(peer_id)` from canonical `network_id + mailbox_secret + peer_id`, decoding `network_id` back to raw 16 bytes before hashing.
- [x] 1.4 Implement atomic file writes and permission enforcement (POSIX `0700` dirs, `0600` files; Windows best-effort restrictive intent).

## 2. Bootstrap Authority

- [x] 2.1 Implement one atomic joined-bootstrap handoff API that persists `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot` as a single success/failure unit.
- [x] 2.2 Implement whole-read / whole-replace roster snapshot behavior and restart-safe reload for every in-scope persisted object.
- [x] 2.3 Treat legacy `internal/pocstate` as reference only; current v1 callers in `02` and `04` must use the new persist APIs.

## 3. Acceptance

- [x] 3.1 Add tests for first-run initialization, single-file rewrite atomicity, bootstrap grouped-write atomicity under failure, restart reload, and permission drift correction.
- [x] 3.2 Prove `02` and `04` can use the new persist authority and `TopicScope` without directly depending on legacy `pocstate`.
