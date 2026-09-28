## Why

The 2026-05-15 Windows/WSL2 real shell validation exposed several product bugs
where otherwise healthy connectivity was blocked or misreported by invite broker
selection, Windows shell target discovery, tmux session discovery, or shell
lifecycle handling.

These fixes should become mainline behavior because they affect real `sh ls`,
`sh_attach`, and invite/join flows across Windows, WSL2, SSH, and Linux PTY
targets.

## What Changes

- Preserve reachable invite broker hostnames in invite codes instead of
  replacing them with resolved IP addresses.
- Decode Windows command output used for WSL target discovery when it is emitted
  as UTF-16LE.
- Classify missing `tmux` on `local`, `wsl:<distro>`, and `ssh:<host>` targets
  as `SH_TMUX_MISSING`.
- Treat an installed `tmux` with no running server as an empty session list, not
  a shell target failure.
- Add a shell protocol completion signal so normal remote `exit`, tmux detach,
  and pane close can end `sh_attach` successfully.
- Treat expected Unix PTY read-close errors, including `/dev/ptmx` EIO after
  child exit, as close semantics resolved by the backend wait result.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-shell-v0`: Clarifies Windows WSL target decoding, tmux
  missing/no-server classification, shell completion signaling, and Unix PTY
  close semantics.
- `miopunch-poc-invite-join-approve-v0`: Clarifies that invite broker
  reachability checks must preserve selected hostname endpoints in emitted
  invite codes.

## Impact

- Affected daemon and shell code:
  - shell target discovery and tmux session listing
  - shell attach acceptor lifecycle handling
  - shell attach task bridge control-frame handling
- Affected invite/join code:
  - reachable invite broker subset selection
  - invite task facts and encoded invite broker endpoints
- Affected public behavior:
  - `sh ls` and `sh_attach` reason codes and completion semantics
  - `miopunch.sh.v0` WebSocket control messages
  - invite code broker endpoint preservation
- Validation impact:
  - Focused Go tests are required for the touched packages.
  - The existing Windows/WSL2 realtest note is supporting evidence.
  - Full `$dev` gates are required before this code-affecting change enters
    mainline.
