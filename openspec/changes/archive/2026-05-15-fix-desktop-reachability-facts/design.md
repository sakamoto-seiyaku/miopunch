## Context

Desktop peer details currently combine three data sources: topology active neighbors, desktop peer sessions, and configured peer metadata. The GUI has placeholders for selected path facts, but the daemon only publishes peer/proto/path/health/activity in session summaries. Real runs show endpoint evidence in connection logs while `/api/v0/desktop/state` and `/api/v0/topology` omit it.

## Goals / Non-Goals

**Goals:**

- Preserve safe selected session endpoint facts in the in-memory session manager.
- Surface the same facts through desktop state and topology active neighbor snapshots.
- Keep direct/public address fields conservative and evidence-driven.
- Make GUI labels distinguish configured reachability hints from observed path facts.

**Non-Goals:**

- Do not infer public/direct addresses from hints, private peer metadata, or log text.
- Do not add a durable historical event store.
- Do not change the wire protocol for peer signaling.
- Do not redesign reachability buckets or neighbor selection.

## Decisions

- Store path facts on `dataplane.SessionSummary`, populated by session construction. This keeps LocalAPI and topology consumers on the existing session-manager boundary instead of scraping task facts or logs.
- Add an optional session-details interface for sessions that can report local/remote endpoints. Wrappers must forward it when they own an underlying session. This avoids widening the core `PeerSession` interface for all implementations at once.
- Record selected endpoint facts at the transport boundary after TLS/QUIC/KCP session establishment. TCP TLS can use the elected net connection; UDP sessions can use the selected UDP local/remote addresses already used by the session.
- Populate `punch_status` from the selected attempt path. `punching_*` becomes `punching`, `direct_*` becomes `direct`, and passive acceptors use their passive attempt path.
- Add selected view/reason to `TopologyAttempt` when the decision response already carries it. This keeps STUN/TCP arbitration evidence product-facing without changing punching behavior.

## Risks / Trade-offs

- [Risk] Endpoint fields may expose private LAN addresses in local-only UI and diagnostics archives. → Mitigation: only expose endpoint facts already visible in daemon logs and LocalAPI diagnostics, and do not expose secrets or raw candidate lists.
- [Risk] Some session implementations may not have endpoint evidence. → Mitigation: keep fields optional and omit unknown values.
- [Risk] Reused sessions can carry stale attempt-level details. → Mitigation: bind facts to the live session object, not the latest task; reused tasks read the same session facts.
