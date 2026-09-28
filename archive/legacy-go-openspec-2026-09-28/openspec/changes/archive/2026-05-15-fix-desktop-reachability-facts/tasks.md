## 1. Dataplane Session Facts

- [x] 1.1 Add a safe path-facts structure and optional reporting interface for peer sessions.
- [x] 1.2 Populate TCP TLS session facts from the elected connection.
- [x] 1.3 Populate UDP session facts from selected local and remote endpoints.
- [x] 1.4 Ensure owned/wrapped sessions preserve wrapped path facts.

## 2. LocalAPI And Topology State

- [x] 2.1 Extend session summaries with optional path facts.
- [x] 2.2 Populate desktop `peer_sessions` entries from summary path facts.
- [x] 2.3 Populate topology active neighbors from the same summary path facts.
- [x] 2.4 Add selected view/reason fields to topology attempts when decision output provides them.

## 3. Desktop GUI

- [x] 3.1 Update peer metadata labels so `v4_hint` and `v6_hint` are clearly shown as hints.
- [x] 3.2 Verify peer details render provided endpoint facts without falling back to `unknown`.

## 4. Validation

- [x] 4.1 Add/update Go tests for session summaries, desktop state, topology state, and selected view evidence.
- [x] 4.2 Run focused tests for touched packages and `openspec validate fix-desktop-reachability-facts --strict`.
- [x] 4.3 Run broader validation required before mainline commit if requested. Not requested for this implementation pass.
