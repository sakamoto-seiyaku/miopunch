## 1. Specification

- [x] 1.1 Create proposal, design, delta spec, and task checklist for portable bundle-local runtime data.

## 2. Implementation

- [x] 2.1 Add bundle path helpers for `data/` and `data/state.json`.
- [x] 2.2 Start desktop-managed daemons with `--session --state_path <bundle>/data/state.json`.
- [x] 2.3 Make manual `miopunch up --session` default to `<bundle>/data/state.json` when no explicit `--state_path` is provided.
- [x] 2.4 Include `data/` and `logs/` directories in generated session bundles and verify archive contents.
- [x] 2.5 Update desktop/package smoke docs to describe local `data/`, reset behavior, and log paths.

## 3. Validation

- [x] 3.1 Add/adjust Go tests for portable data path resolution and session state path defaults.
- [x] 3.2 Run focused Go tests for `internal/bundlepath`, `internal/desktopbridge`, and `cmd/miopunch`.
- [x] 3.3 Run OpenSpec strict validation for this change.
- [x] 3.4 Rebuild and extract current Linux/Windows session bundles for real-machine smoke.
