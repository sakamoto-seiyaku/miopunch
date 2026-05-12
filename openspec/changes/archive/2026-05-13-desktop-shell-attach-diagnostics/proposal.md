## Why

The desktop shell path now reaches real cross-platform attach attempts, but the
current failure surface collapses late shell attach faults into generic
`EOF`/WebSocket `1006` outcomes. That blocks efficient debugging because the
operator cannot tell whether the failure came from the desktop bridge, LocalAPI
attach path, remote shell target startup, or the terminal/session layer.

## What Changes

- Preserve shell attach failure causes across the desktop shell chain instead of
  reducing them to generic `EOF` or WebSocket `1006` results when richer
  information is available.
- Make post-attach `sh_attach` failures and abnormal remote shell termination
  produce stable, actionable diagnostics that identify the failing layer.
- Improve the desktop shell UI so attach failures and unexpected shell closure
  show a short operator-facing summary while keeping retry on the same peer,
  target, and session.
- Add structured logging around the shell attach path with clear `debug`,
  `info`, `warn`, and `error` intent so reruns can pinpoint the failing hop.
- Keep scope limited to Linux desktop -> Windows shell attach diagnostics; do
  not fold in the separate MQTT broker timeout issue in this change.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `miopunch-desktop-gui-v0`: require the desktop shell view to surface
  meaningful attach/close diagnostics instead of only generic disconnect text.
- `miopunch-poc-shell-v0`: require late `sh_attach` failures and abnormal remote
  shell termination to preserve stable shell reason codes and layer-specific
  diagnostics.
- `miopunch-poc-output-contract-v0`: require interactive shell failures that
  happen after attach setup to still produce actionable stage/reason/facts
  output for operators.

## Impact

- Affected code: `cmd/miopunch-desktop/frontend/dist`,
  `internal/desktopbridge`, `internal/localapi`, `internal/task`,
  `internal/pocacceptor`, and Windows shell target code under
  `internal/shelltarget`.
- Public behavior: desktop shell failures become explainable and retryable
  instead of ending as opaque disconnects.
- APIs/protocols: no new task kinds and no new shell WebSocket subprotocol are
  planned; this change tightens diagnostics on existing `sh_attach` behavior.
- Validation: focused shell attach tests/log assertions for touched paths during
  implementation; full `$dev` gates apply before code-affecting changes enter
  mainline.
