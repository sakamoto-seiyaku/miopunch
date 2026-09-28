## 1. OpenSpec Foundation

- [x] 1.1 Create the proposal, design, spec deltas, and implementation checklist for `desktop-runtime-state-refresh-foundation`.

## 2. LocalAPI Desktop State Contract

- [x] 2.1 Add shared desktop runtime state and desktop event types for snapshot and typed revisioned updates.
- [x] 2.2 Extend the task/runtime owner to assemble desktop runtime snapshots and publish desktop state updates with monotonic revisions.
- [x] 2.3 Add `GET /api/v0/desktop/state` and `GET /api/v0/desktop/events` plus LocalAPI client support and SSE parsing tests.

## 3. Desktop Bridge Runtime Path

- [x] 3.1 Add desktop bridge methods for runtime start and runtime resync and relay `desktop:state` / `desktop:runtime` events.
- [x] 3.2 Move desktop startup away from eager backend `Connect()` so live runtime bootstrap happens through the new single path.

## 4. Desktop Frontend Store

- [x] 4.1 Replace the current connect-plus-refresh bootstrap path with a single desktop runtime store bootstrap and resync flow.
- [x] 4.2 Apply typed desktop runtime updates in the committed frontend assets and keep task-only APIs for compatibility/detail flows.
- [x] 4.3 Update fake bridge/runtime fixtures and browser tests to cover the new desktop state contract.

## 5. Validation

- [x] 5.1 Run focused Go tests for LocalAPI, desktop bridge, and task/runtime state changes.
- [x] 5.2 Run desktop browser tests for runtime bootstrap, resync, and live state updates.
