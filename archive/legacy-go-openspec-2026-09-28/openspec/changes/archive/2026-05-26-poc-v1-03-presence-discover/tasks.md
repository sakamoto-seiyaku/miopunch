## 1. Contracts And Inputs

- [x] 1.1 Add `internal/pocv1/presence` domain contracts for `Observation`, `DiscoverView`, `DiscoverPeer`, and `LastSeenPeer`.
- [x] 1.2 Load `runtime_broker`, `TopicScope`, `roster_snapshot`, and device-key-derived `self_peer_id` from 06 inputs; accept caller-supplied local `device_name` / `platform` / `app_ver`.
- [x] 1.3 Implement the fixed JSON payload and the `peer_id` keyed observation parser, including diagnostic-only rejection for malformed / unsupported / topic-mismatch payloads.

## 2. Lifecycle And Merge

- [x] 2.1 Implement publish/subscribe logic for `mp/v1/net/<net_root>/presence/<peer_id>` with retained `online`, graceful retained `offline`, and retained LWT `offline` on the same topic.
- [x] 2.2 Build `DiscoverView` by joining trusted remote roster entries with `.../presence/+` observations; exclude `self_peer_id`, default to `offline`, and let roster display hints win over non-empty presence hints.
- [x] 2.3 Define the minimal `LastSeenPeer` object model and in-memory merge semantics in 03; do not add persist file roles or 06 foundation APIs in this change.

## 3. Acceptance

- [x] 3.1 Add tests for retained snapshot hydration, roster-only offline rows, graceful shutdown offline, reconnect updates, unexpected-disconnect LWT, and duplicate retained/live convergence.
- [x] 3.2 Add a local MQTT smoke and consumer seam tests proving 04 reads only `DiscoverPeer.online_state` while 07 projects the same `DiscoverView` into runtime DTOs without any extra directory query or legacy re-join logic.
