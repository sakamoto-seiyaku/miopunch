## 1. Presence Runtime

- [ ] 1.1 Add `internal/pocv1/presence` and implement publish/subscribe logic for `mp/v1/net/<net_root>/presence/<peer_id>`.
- [ ] 1.2 Implement retained `online` publish and retained LWT `offline` configuration on the same topic.
- [ ] 1.3 Implement the fixed JSON payload and the `peer_id` keyed observation parser.

## 2. Snapshot Consumption

- [ ] 2.1 Build the discover view by joining `.../presence/+` online/offline state with the persisted `roster_snapshot`.
- [ ] 2.2 Define the minimal `last_seen_peers` object model in `03`; only wire persistence after that model is frozen instead of inventing it inside `06` foundation.
- [ ] 2.3 Keep presence observation-only; do not route trust, remote `x25519`, inbox authority, or dial state through it.

## 3. Acceptance

- [ ] 3.1 Add tests for retained snapshot hydration, reconnect updates, and offline LWT behavior.
- [ ] 3.2 Add a local MQTT smoke proving a consumer can see peer online state from presence and resolve trusted member identity from the persisted roster without any extra directory query.
