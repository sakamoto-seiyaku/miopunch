## Why

Current `miopunch sh ls <peer>` lists every discoverable shell target, but it
does not tell the operator which targets can actually support the tmux-backed
attach and recovery flow. In real Windows environments this produces long
`ssh:*` and `wsl:*` lists where some entries are only aliases or enumerated
distros, not confirmed shell targets.

We need an explicit way to ask for "ready for tmux attach" targets without
turning the default list operation into a slow or interactive probe across
every SSH alias.

## What Changes

- Add an explicit `miopunch sh ls <peer> --ready` mode that probes each
  discoverable shell target and returns only targets confirmed ready for
  tmux-backed attach/recovery.
- Keep the default `miopunch sh ls <peer>` behavior as fast raw enumeration of
  all discoverable targets.
- Define readiness as non-interactive tmux preflight success, not the presence
  of an already-running tmux session.
- Require ready probing to be bounded and partial-success tolerant: targets
  that cannot be confirmed quickly become structured `unknown` / diagnostic
  results instead of failing the entire command.
- Preserve the existing `miopunch sh ls <peer> <target>` meaning for tmux
  session listing on a chosen target.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `miopunch-poc-shell-v0`: extend `sh_ls` with an explicit ready-probe mode,
  ready/unsupported/unknown target classification, and partial-success shell
  diagnostics for target readiness probing.

## Impact

- Affected code: `cmd/miopunch` shell-list argument parsing, current v1 runtime
  shell-list request/response plumbing, and `internal/shelltarget` probe logic
  for Linux local, Windows WSL, and Windows SSH targets.
- Affected behavior: operator-facing `sh ls` output, JSON/report evidence, and
  shell-target diagnostics for explicit readiness probing.
- No change to `sh_attach` session semantics, tmux recovery semantics, or the
  default all-target enumeration behavior.
