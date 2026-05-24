## 1. Persist Foundation

- [ ] 1.1 Add `internal/pocv1/persist` and implement the `device/` + `networks/<canonical network_id>/` layout resolver.
- [ ] 1.2 Implement typed stores for device keys, `self_member_credential`, `mailbox_secret`, single runtime broker config, `roster_snapshot`, `last_seen_peers`, and `ui_state`.
- [ ] 1.3 Implement `TopicScope` derivation for `net_root`, `presence_topic(peer_id)`, and `inbox_topic(peer_id)` from canonical `network_id + mailbox_secret + peer_id`, decoding `network_id` back to raw 16 bytes before hashing.
- [ ] 1.4 Implement atomic file writes and permission enforcement (`0700` dirs, `0600` files).

## 2. Migration Authority

- [ ] 2.1 Treat legacy `internal/pocstate` as reference only; current v1 callers must use the new persist APIs.
- [ ] 2.2 Add restart-safe reload behavior for every persisted object.

## 3. Acceptance

- [ ] 3.1 Add tests for first-run initialization, rewrite atomicity, restart reload, and permission drift correction.
- [ ] 3.2 Prove `02`, `03`, `04`, and `07` can use the new persist authority and `TopicScope` without directly depending on legacy `pocstate`.
