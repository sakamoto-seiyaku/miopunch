## Context

The desktop GUI already has a snapshot-first runtime foundation through `GET /api/v0/desktop/state`, `GET /api/v0/desktop/events`, Wails `DesktopRuntimeStart`, `DesktopRuntimeResync`, and `SaveDesktopConfig`. The confirmed frontend prototype changes the product shape from an Access/task-centric POC into a control console with `Network / Shell / Admin / Settings`.

The backend state model already carries topology, tasks, peer sessions, shell sessions, config, diagnostics, and approval requests. The missing work is to make that existing runtime contract rich enough for live mode, then remove prototype-only fallbacks from the frontend.

## Goals / Non-Goals

**Goals:**

- Keep one authoritative desktop runtime state contract instead of adding a second GUI-specific API.
- Persist local peer aliases as desktop-local preferences.
- Expose remote member display hints from existing approved member declaration fields.
- Make Shell Resume explicit and safe by exposing whether a shell task is locally attachable.
- Move invite/approval UI semantics from Access into Admin while preserving Access deep-link compatibility.
- Ensure live-mode Network/Shell/Admin rendering uses daemon state or explicit unknown/not measured values.

**Non-Goals:**

- Do not write aliases to governance declarations or synchronize them to other peers.
- Do not define true long-lived remote shell persistence as part of Resume v1.
- Do not build continuous RTT, throughput, or packet-loss telemetry.
- Do not add another state-file reader or frontend-only LocalAPI bypass.
- Do not redesign the full Settings editor beyond fields needed by this console contract.

## Decisions

### Extend the existing desktop runtime contract

Use the current desktop state/config endpoints and Wails bridge methods. `DesktopPreferences`, `TopologyMember`, `DesktopShellSession`, and peer/topology session summaries are extended in place.

Alternative considered: add new console-only endpoints. Rejected because the existing desktop runtime state is already snapshot-first, revisioned, streamed, and product-facing.

### Store peer aliases as desktop-local preferences

Aliases are saved under desktop settings/preferences and returned through desktop config snapshots. They remain keyed by peer ID and never replace peer ID or remote member name.

Alternative considered: store aliases in governance or member declarations. Rejected because aliases are local operator labels, not network-wide identity claims.

### Use approved member declarations for remote display metadata

Topology member projection reads `member_name` and `platform` from existing `approve_member` decl bodies when present. UI display precedence is alias, then remote member name, then shortened peer ID.

Alternative considered: infer names from frontend preview tables or known-peer config. Rejected because live mode needs daemon-owned, persisted identity hints.

### Keep Shell Resume to local attach recovery

Resume v1 means reusing an existing local `sh_attach` task only while it can accept or hold the foreground WebSocket attach. `DesktopShellSession.attachable` controls whether Resume is enabled.

Alternative considered: make Resume imply remote background shell persistence. Rejected because remote tmux/session persistence exists below the task layer, but daemon task/WS attach persistence is not yet designed for long-lived disconnected clients.

### Treat path detail fields as optional evidence

Expose only reliable real values from topology/session summaries. Missing RTT, throughput, loss, endpoints, or punch details must render as unknown/not measured instead of preview data.

Alternative considered: keep frontend preview metrics in live mode. Rejected because it makes diagnostics look real when the daemon has not measured them.

## Risks / Trade-offs

- [Risk] Shell attach lifecycle touches shared manager state and channels. -> Mitigation: implementation must follow `$go-concurrency`, avoid blocking sends under locks, and publish shell-session updates on lifecycle transitions.
- [Risk] Existing frontend tests and old Access wording may conflict with the new navigation. -> Mitigation: update browser tests around Network/Shell/Admin/Settings and retain only Access deep-link redirect behavior.
- [Risk] Optional path fields can appear sparse at first. -> Mitigation: v1 requires honest unknown/not measured rendering and leaves richer telemetry for a later change.
- [Risk] Alias persistence can accidentally mask identity. -> Mitigation: keep Peer ID visible and expose remote name separately wherever alias is shown.

## Migration Plan

1. Extend backend desktop state/config types and persistence.
2. Update LocalAPI and Wails bridge contract tests for new fields.
3. Update frontend live-mode store/rendering to consume aliases, member names, attachable shell sessions, and optional path fields.
4. Update browser tests for navigation, role gating, Admin approval flows, alias save, Shell Resume, and no preview fallback in live mode.
5. Run OpenSpec validation, focused Go tests, focused frontend tests, and the full `$dev` gate before mainline code-affecting merge.

Rollback is normal code rollback: the fields are additive and optional, so older frontend/backend combinations should continue to tolerate missing values.

## Open Questions

- None for this change. Continuous connection telemetry and true backend shell persistence are intentionally deferred.
