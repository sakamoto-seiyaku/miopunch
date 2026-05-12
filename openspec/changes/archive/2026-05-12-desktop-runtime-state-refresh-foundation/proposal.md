## Why

`miopunch-desktop` still behaves like a task launcher that stitches together several bridge calls and task events to approximate live runtime state. That keeps the UI in a wrapped-tool posture instead of a native control console, and it leaves refresh, reconnect, and long-running runtime changes harder to reason about than they need to be.

## What Changes

- **BREAKING**: replace the desktop GUI's piecemeal runtime bootstrap (`Connect` plus separate `GetStatus` / `GetPeers` / `GetTopology` / `GetTasks`) with one authoritative desktop runtime state contract.
- Add LocalAPI desktop runtime endpoints for a full typed snapshot and a revisioned snapshot-first event stream.
- Add a desktop state model that carries product-facing runtime state, not just task internals: status, topology, peer sessions, config, diagnostics, shell sessions, and task history.
- Update the Wails desktop bridge to start and resync the desktop runtime through one path, relay typed desktop state events, and keep task-only APIs as compatibility/debug surfaces.
- Rework the desktop frontend to consume the new state contract as a long-lived store so the main UI follows runtime state directly instead of depending on manual refresh or task event side effects.

## Capabilities

### New Capabilities
<!-- None. -->

### Modified Capabilities
- `miopunch-poc-localapi-v0`: extend the LocalAPI route set and stream contract with a product-facing desktop runtime state snapshot and revisioned desktop event stream.
- `miopunch-desktop-gui-v0`: switch desktop runtime bootstrap and live refresh to the new authoritative desktop state contract and move primary UI state away from task-event stitching.

## Impact

- LocalAPI server/client code under `internal/localapi`.
- Desktop runtime state assembly and event publication in task/runtime-owned packages.
- Wails desktop bridge and lifecycle code under `cmd/miopunch-desktop`.
- Desktop committed frontend assets and browser tests under `cmd/miopunch-desktop/frontend`.
