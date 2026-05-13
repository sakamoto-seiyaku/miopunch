## Context

The desktop runtime snapshot already includes `config` and `diagnostics`, and
the frontend consumes revisioned `config.replace` and `diagnostics.replace`
events. Today those fields are mostly read-only summaries: Settings cannot save
runtime config through LocalAPI, cannot persist desktop preferences, and cannot
export a useful redacted bundle for real machine troubleshooting.

The current session-bundle model keeps state under `data/` and logs under
`logs/` beside the extracted binaries. The daemon remains the authoritative
owner of runtime/config state; the frontend must use Wails/LocalAPI bridge
methods instead of reading or editing files directly.

## Goals / Non-Goals

**Goals:**

- Add the smallest daemon-authoritative Settings write path for current runtime
  fields.
- Show desired and effective config in desktop runtime state.
- Persist desktop-only preferences in a bundle-local desktop settings file.
- Export redacted runtime diagnostics from the bridge to a user-selected archive.
- Keep config/diagnostics updates revisioned through the existing desktop state
  stream.

**Non-Goals:**

- Do not add sing-specific settings in this change.
- Do not build a complete generic config editor.
- Do not mutate active peer or shell sessions when settings change; report that
  network changes apply to future connections or require reconnect.
- Do not expose secrets, invite codes, private keys, or raw membership material
  in desktop state or exported diagnostics.

## Decisions

### Use an additive desktop config model

Extend the existing `DesktopConfig` shape instead of replacing it. Keep
`local`, `known_peers`, and `net`, then add `desired`, `effective`,
`preferences`, and apply metadata.

Rationale: existing UI/tests already consume the old fields, and additive state
keeps older clients compatible.

Alternative considered: introduce a new `/desktop/settings` snapshot. Rejected
because Settings is part of the product desktop runtime contract and should stay
inside the authoritative runtime snapshot.

### Save through `PATCH /api/v0/desktop/config`

Add one versioned LocalAPI route that accepts partial desktop config updates and
returns the fresh desktop snapshot. The daemon validates, persists, applies
immediate settings, and emits `config.replace` plus diagnostics updates.

Rationale: one route gives the desktop a simple save/resync path without making
the browser own file formats or merge policy.

Alternative considered: model each setting as a task kind. Rejected because
config save is a short daemon control operation, not a long-running workflow.

### Split network state and desktop-only preferences

Network settings continue to live in `data/state.json`; desktop-only preferences
live in `data/desktop_settings.json`. Log level is a daemon preference loaded on
startup and applied immediately with the existing logger.

Rationale: current network state already has the right persistence and sharing
semantics; shell defaults and log level are local desktop/operator preferences.

Alternative considered: put desktop preferences into `state.json`. Rejected
because it would mix POC network state with UI/operator preferences and make
membership propagation boundaries less clear.

### Export a redacted zip archive

The bridge writes a `.zip` containing JSON snapshots and available logs. State
files are summarized through typed runtime state, not copied raw. Logs and JSON
are line/value redacted before writing.

Rationale: real support/debug flows need one shareable artifact, but raw state
can contain secrets.

Alternative considered: export Markdown only. Rejected because logs and runtime
snapshots are both needed for useful diagnostics.

## Risks / Trade-offs

- [Risk] Config updates can imply more than the daemon can apply live. ->
  Mitigation: mark network settings as future-connection-only when sessions are
  active and make that visible in Settings.
- [Risk] Diagnostics export may miss future files. -> Mitigation: export the
  authoritative runtime snapshot plus known bundle logs first; add files
  explicitly as new diagnostic sources become stable.
- [Risk] Hand-maintained static JS can regress unrelated UI paths. ->
  Mitigation: keep the Settings DOM changes localized and expand Playwright
  coverage.

## Migration Plan

1. Add OpenSpec spec deltas and validate them.
2. Add task-manager desktop settings persistence, config update, and diagnostics
   state.
3. Add LocalAPI route/client and Wails bridge methods.
4. Update Settings UI and fake bridge tests.
5. Run focused Go and Playwright validation.

Rollback: remove the new route/bridge methods and frontend Settings controls.
Existing desktop runtime state remains compatible because old fields are
preserved.

## Open Questions

- None. Sing settings are intentionally deferred.
