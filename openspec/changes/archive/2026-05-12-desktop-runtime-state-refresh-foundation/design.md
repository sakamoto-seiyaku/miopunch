## Context

The current desktop runtime path is split across multiple layers:

- the Wails app calls `Connect()` during backend startup
- the frontend connects again after loading
- snapshot refresh pulls `status`, `peers`, `topology`, and `tasks` through separate bridge calls
- task SSE keeps task cards warm, but non-task runtime state is still effectively refreshed by refetch

That architecture was acceptable for the POC launcher phase, but it does not match the control-console direction recorded in `docs/notes/2026-05-11-desktop-control-console-live-refresh.md`. The next stable step is not a bigger refetch coordinator. It is a first-class desktop runtime state API that the shell can bootstrap once, stream continuously, and resync only when revisions are missed or the operator explicitly requests a refresh.

Constraints:

- `miopunch-desktop` remains a Wails shell for Windows/Linux.
- LocalAPI remains IPC-only and the daemon remains the authoritative runtime/config source.
- Existing task APIs, reports, and shell transport stay available for compatibility and debug flows.
- The current frontend is committed static JS in `frontend/dist`, so the new runtime store should be plain JS and not depend on a new frontend toolchain.

## Goals / Non-Goals

**Goals:**

- Introduce one authoritative LocalAPI desktop runtime state contract: full snapshot plus typed revisioned updates.
- Make the desktop bridge expose one runtime bootstrap/resync path and one desktop state event channel.
- Make the desktop frontend apply ordered updates into a single store so the main UI follows product state directly.
- Preserve current task APIs and task SSE as compatibility/debug surfaces while removing them from the primary runtime data path.

**Non-Goals:**

- Do not replace Wails with another desktop shell.
- Do not redesign the Access approval workflow in this change; `approval_requests` may remain structurally present but minimally populated until the workflow-specific follow-up change lands.
- Do not introduce a generic JSON Patch protocol. Typed domain updates are preferred and sufficient.
- Do not add a new frontend build system or framework migration.

## Decisions

### 1. Introduce a dedicated desktop runtime state API in LocalAPI

- Add `GET /api/v0/desktop/state` for full bootstrap/resync.
- Add `GET /api/v0/desktop/events` for snapshot-first SSE.
- Return a typed `DesktopStateSnapshot` with a monotonic `rev`.

Why:

- The current `status` / `peers` / `topology` / `tasks` split makes the desktop client own consistency across several calls.
- A single snapshot route gives the client an atomic bootstrap/resync point.

Alternative considered:

- Keep the current routes and build only a bridge-side aggregator.
- Rejected because it leaves LocalAPI without a first-class desktop state contract and keeps other clients from sharing the same product-facing model.

### 2. Use typed replacement/update events with revision tracking

- Desktop events carry `kind`, `base_rev`, `rev`, and one typed payload.
- The first desktop event is always `snapshot`.
- Subsequent events are explicit typed updates such as `task.upsert`, `topology.replace`, `config.replace`, `peer_sessions.replace`, `shell_sessions.replace`, `diagnostics.replace`, and `approval_requests.replace`.

Why:

- Typed events are easier to validate, test, and evolve than an open-ended patch DSL.
- Revisions let the client detect missed state and fall back to one full resync.

Alternative considered:

- Coarse invalidation events plus debounce/refetch.
- Rejected because the user explicitly wants the architecture change to happen once, not another intermediate refetch layer.

Alternative considered:

- Generic JSON Patch / merge-patch updates.
- Rejected because the state surface is small enough for typed updates and the extra patch semantics would add long-term compatibility burden.

### 3. Assemble desktop state close to the runtime owner

- Add a dedicated internal package for desktop state types.
- Keep snapshot assembly and revisioned desktop event publication next to the runtime/task manager rather than inside the desktop bridge.
- Let LocalAPI serve the state model directly from the daemon-side owner.

Why:

- The daemon/task manager already owns topology, task state, peer config, and session summaries.
- Putting assembly there avoids a second "desktop-only truth" in the bridge.

Alternative considered:

- Compute desktop state inside the Wails bridge by combining existing LocalAPI calls.
- Rejected because it keeps the bridge as a custom client-side aggregator instead of making the daemon expose a product contract.

### 4. Make the desktop bridge lifecycle explicit and single-path

- Stop auto-connecting in backend startup.
- Add `DesktopRuntimeStart()` to connect, bootstrap the desktop state, and start the desktop event pump.
- Add `DesktopRuntimeResync()` to fetch one fresh full snapshot.
- Emit `desktop:state` for typed desktop updates and `desktop:runtime` for stream lifecycle transitions.

Why:

- The frontend must own when it starts listening and when it asks for the first runtime snapshot.
- This removes the current startup race where Go can begin runtime work before the UI listener graph is ready.

Alternative considered:

- Keep startup auto-connect and merely swap the stream payload.
- Rejected because it preserves the existing double-bootstrap shape.

### 5. Keep task routes as compatibility/debug surfaces, not the primary runtime path

- The main desktop UI store consumes `DesktopRuntimeStart`, `DesktopRuntimeResync`, and `desktop:state`.
- Existing task getters/events remain for task details, invite-code fallback, reports, and compatibility.

Why:

- The current UI still needs task details and report export.
- Moving the main console away from task stitching does not require removing those APIs.

Alternative considered:

- Remove task SSE immediately.
- Rejected because task debug/history flows still benefit from the existing task-specific stream.

## Risks / Trade-offs

- [Risk] Desktop snapshot assembly may become a kitchen sink model. -> Mitigation: keep the type product-facing and grouped by stable domains, and reserve workflow-specific richness for follow-up changes.
- [Risk] Session/runtime changes that bypass task completion could be missed if not wired into desktop event publication. -> Mitigation: publish replacement events from session manager lifecycle transitions in addition to task-driven updates.
- [Risk] The committed static frontend is large and hand-maintained. -> Mitigation: confine the JS change to one store/bootstrap path and keep browser tests authoritative for behavior.
- [Risk] Approval request state is not fully productized yet. -> Mitigation: include the subtree and event kind now, but allow it to remain minimally populated until the workflow change lands.

## Migration Plan

1. Add the desktop state types, LocalAPI endpoints, and manager-side desktop subscriptions.
2. Add the LocalAPI client and desktop bridge methods/events while retaining existing task/state bridge methods for compatibility.
3. Rework the desktop frontend bootstrap to use `DesktopRuntimeStart()` and `desktop:state`.
4. Update browser and Go tests to assert the new runtime path.
5. Keep manual Refresh as an explicit resync path rather than a per-slice fetch chain.

Rollback:

- The old bridge methods and task routes remain available during this change, so reverting the frontend to the old bootstrap path remains possible until the new behavior is validated.

## Open Questions

- None for this implementation pass. The follow-up Access approval workflow change will decide how rich `approval_requests` becomes once the UI stops using manual approve-code entry.
