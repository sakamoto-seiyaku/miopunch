## 1. OpenSpec

- [x] 1.1 Add proposal, design, task list, and delta specs.
- [x] 1.2 Run `openspec validate --all --strict --no-interactive`.

## 2. Runtime Config State

- [x] 2.1 Add desktop settings persistence under `data/desktop_settings.json`.
- [x] 2.2 Extend desktop runtime config with desired/effective/preference/apply metadata while preserving existing fields.
- [x] 2.3 Add task-manager config update validation and persistence for current runtime fields.
- [x] 2.4 Apply persisted log level on daemon startup and immediate log-level updates on save.
- [x] 2.5 Publish `config.replace` and `diagnostics.replace` events after successful config saves.

## 3. LocalAPI And Bridge

- [x] 3.1 Add `PATCH /api/v0/desktop/config`.
- [x] 3.2 Add LocalAPI client support for the config update route.
- [x] 3.3 Add Wails bridge methods for saving runtime config and exporting diagnostics.
- [x] 3.4 Write redacted diagnostics zip archives from runtime snapshots and available bundle logs.

## 4. Desktop UI

- [x] 4.1 Update Settings navigation and forms for Runtime Config, Local Daemon, Diagnostics, and Preview.
- [x] 4.2 Render desired/effective config, apply scope, validation errors, and export status.
- [x] 4.3 Update fake bridge fixtures and Playwright coverage for config save, validation, runtime updates, and diagnostics export.

## 5. Validation

- [x] 5.1 Run focused Go tests for `internal/task`, `internal/localapi`, and `cmd/miopunch-desktop` with desktop tags where needed.
- [x] 5.2 Run desktop Playwright tests.
- [x] 5.3 Run broader host validation appropriate for a code-affecting change before mainline integration.
