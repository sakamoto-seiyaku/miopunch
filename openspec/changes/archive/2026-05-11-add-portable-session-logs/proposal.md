## Why

Real Windows/Linux session smoke now depends on portable bundles, but runtime
diagnostics are still hard to collect because desktop and daemon logs are not
written beside the extracted binaries. When true-machine tests fail, operators
need local log files and a short ordered smoke path without guessing where
state or output landed.

## What Changes

- Write desktop session runtime logs into a `logs/` directory in the extracted
  session bundle.
- Write desktop-managed `miopunch up --session` daemon logs into the same local
  `logs/` directory.
- Keep the current portable/session bundle shape; do not reintroduce installer,
  Administrator, root, or system service requirements.
- Expand bundled smoke instructions with local log paths and a simple
  Windows/Linux manual test sequence from launch through invite/join, ping, and
  shell attach.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-packaging-v0`: current portable/session bundles shall
  provide local runtime log paths and practical smoke instructions for manual
  Windows/Linux testing.

## Impact

- Runtime startup: `cmd/miopunch`, `cmd/miopunch-desktop`.
- Shared runtime path helper: `internal/...` as needed.
- Packaging docs/scripts: session bundle `SMOKE.md` generation and desktop
  smoke notes.
- Validation: code-affecting change; run focused Go tests/builds and rebuild
  Linux/Windows session bundles before true-machine testing.
