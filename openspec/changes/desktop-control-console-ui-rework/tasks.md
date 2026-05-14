## 1. Backend Desktop Runtime Contract

- [x] 1.1 Add `peer_aliases` to desktop preferences and preferences updates, including normalization, persistence, clearing empty aliases, and snapshot/config-event output.
- [x] 1.2 Add LocalAPI and task-manager tests proving `PATCH /api/v0/desktop/config` persists aliases in desktop settings and does not alter governance, decls, or known-peer config.
- [x] 1.3 Add `member_name` and `platform` to topology member projection from approved member declarations, with tests for present and missing display hints.
- [x] 1.4 Add optional path detail fields to the appropriate desktop topology or peer-session summaries using only reliable daemon evidence; omit unknown values.

## 2. Shell Attachability

- [x] 2.1 Add `attachable` to desktop shell-session summaries and derive it from the local `sh_attach` task attach lifecycle.
- [x] 2.2 Publish `shell_sessions.replace` when shell attachability changes, including task creation, WebSocket attach, timeout, close, and task completion paths.
- [x] 2.3 Add focused Go tests for attachable waiting sessions, non-attachable completed sessions, and shell-session event publication.
- [x] 2.4 Review shell attach lifecycle changes with `$go-concurrency`, ensuring no blocking channel sends occur under manager locks.

## 3. Frontend Console Rework

- [x] 3.1 Update primary navigation to `Network`, `Shell`, `Admin`, and `Settings`, with `Access` deep links redirecting to Network.
- [x] 3.2 Update role gating so first-run empty nodes land on Network Join, joined members see Network/Shell only, and owner/admin nodes see Admin.
- [x] 3.3 Move invite creation, approval review, and approval decisions into Admin while keeping the existing bridge task kinds.
- [x] 3.4 Save Network local aliases through `SaveDesktopConfig` and render aliases from desktop preferences after snapshots/events apply.
- [x] 3.5 Render live device names from alias, `member_name`, then peer ID; remove live-mode fallback to preview device names.
- [x] 3.6 Render live path details only from daemon snapshot fields, showing unknown or not measured when values are absent.
- [x] 3.7 Gate Shell Resume on `DesktopShellSession.attachable`; keep non-attachable sessions visible but require opening another shell.
- [x] 3.8 Add a Settings-only first-run Owner/Admin mode switch that reveals Admin without changing the Network Join flow.
- [x] 3.9 Auto-start explicit approval review after Admin invite creation returns an invite code.
- [x] 3.10 Rebalance Network topology/detail cards so identity, metrics, path facts, node facts, and diagnostics are visually separated.
- [x] 3.11 Rework Shell target/session panels so target discovery stays in choices, sessions have Resume/New controls, and Zen mode maximizes the terminal.

## 4. Tests and Validation

- [x] 4.1 Update frontend browser tests for primary navigation, Access deep-link redirect, role gating, Admin approval flows, alias save, and Shell Resume attachability.
- [x] 4.2 Update frontend fake desktop runtime fixtures to include `peer_aliases`, `member_name`, `platform`, `attachable`, and optional path fields.
- [x] 4.3 Run `rtk openspec validate desktop-control-console-ui-rework --strict --no-interactive`.
- [x] 4.4 Run focused Go tests for desktop settings, topology member projection, LocalAPI desktop config, and shell-session summaries.
- [x] 4.5 Run focused frontend browser tests under `cmd/miopunch-desktop/frontend`.
- [x] 4.6 Add focused browser coverage for enabling first-run Owner/Admin mode from Settings.
- [x] 4.7 Add focused browser coverage for invite auto-listener and Shell target/session panel behavior.
- [x] 4.8 Before mainline code-affecting merge, run the full `$dev` validation gate.
