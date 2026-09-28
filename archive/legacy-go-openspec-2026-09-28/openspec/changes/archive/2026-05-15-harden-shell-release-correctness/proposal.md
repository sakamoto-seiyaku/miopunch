## Why

The real Windows/WSL2 shell validation and follow-up review found a few
release-facing correctness risks that remain after the first regression batch:
session names can be misreported on Windows WSL targets, Windows SSH tmux
commands are inconsistent, shell attach readiness still depends on a fixed
time window, and invite broker helper naming still leaves a path for
reintroducing IP canonicalization into invite codes.

These are narrow hardening fixes for existing behavior. They should be handled
before release because they affect correctness and future maintainability, but
they do not require new shell modes, daemon lifecycle design, or protocol
expansion.

## What Changes

- Parse default `tmux list-sessions` output on Windows `wsl:<distro>` targets
  so `sh ls` emits clean session names even without `-F "#S"`.
- Make Windows `ssh:<host>` tmux list/preflight command construction consistent
  with attach by avoiding a remote `--` command token.
- Replace shell attach's fixed readiness sleep with deterministic bridge setup
  and immediate failure polling before sending attach-ready.
- Clarify invite broker helper boundaries so invite-code output continues to
  preserve reachable hostname endpoints and future code does not reuse an
  IP-canonicalizing helper for emitted invite brokers.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-shell-v0`: Hardens session discovery, Windows SSH tmux command
  construction, and shell attach readiness semantics.
- `miopunch-poc-invite-join-approve-v0`: Clarifies that invite-code broker
  emission uses a hostname-preserving path and must not use IP canonicalization.

## Impact

- Affected shell code:
  - Windows WSL and SSH tmux session listing/preflight.
  - Shell attach acceptor setup and readiness timing.
- Affected invite/join code:
  - Broker helper naming/documentation and invite-code endpoint selection tests.
- Public behavior:
  - `sh ls <peer> wsl:<distro>` session facts become clean session names.
  - `sh_attach` setup failure vs ready semantics become less timing-sensitive.
  - Invite codes continue to preserve selected reachable hostname broker
    endpoints.
- Validation impact:
  - Focused Go tests are required for shell target parsing/command construction,
    shell attach readiness, and invite broker endpoint preservation.
  - Full `$dev` gates are required before this code-affecting change enters
    mainline.
