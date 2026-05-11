## Why

Portable/session bundles currently write logs beside the extracted binaries, but runtime state still defaults to the user config directory. Real-machine smoke runs can therefore reuse identity, network, peers, reports, and invite state from an earlier extraction, which makes restart and reinstall testing unreliable.

## What Changes

- Store portable/session runtime data under the extracted bundle directory in `data/`.
- Start desktop-managed daemons with an explicit bundle-local `--state_path`.
- Make manual `miopunch up --session` prefer the same bundle-local state path when no `--state_path` override is supplied.
- Update generated smoke instructions and desktop docs to identify both `logs/` and `data/`.
- Keep explicit `--state_path` overrides working for labs and debugging.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-packaging-v0`: portable/session bundles shall keep runtime data beside the extracted binaries under `data/`, not in the user's global config directory.

## Impact

- Session bundle path helpers under `internal/bundlepath`.
- Desktop daemon bootstrap under `internal/desktopbridge`.
- `miopunch up --session` startup path for Linux and Windows.
- Generated `SMOKE.md` and desktop packaging documentation.
- Session bundle tests and focused Go validation.
