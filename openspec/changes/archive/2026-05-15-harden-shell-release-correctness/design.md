## Context

The previous shell regression batch fixed the real Windows/WSL2 failures that
blocked current shell flows. Follow-up review found a smaller set of correctness
risks that remain close to release: one compatibility workaround can emit
incorrect tmux session names, Windows SSH tmux command construction is
inconsistent with attach, shell attach readiness is still timing-based, and the
invite broker helper surface still includes an IP-canonicalizing helper that is
unsafe for invite-code output.

The target is release hardening, not new shell product design. The current
interactive `shell_exit` semantics and `tmux new -A -s <session>` reuse behavior
remain unchanged.

## Goals / Non-Goals

**Goals:**

- Make `sh ls <peer> wsl:<distro>` report clean tmux session names when using
  default tmux output.
- Keep Windows SSH tmux list/preflight command construction aligned with the
  attach command shape.
- Remove the fixed 200ms attach-ready timing window without changing successful
  attach lifecycle behavior.
- Make invite broker helper boundaries explicit so emitted invite codes preserve
  reachable hostname endpoints.
- Add focused tests for each hardening concern.

**Non-Goals:**

- Do not add `sh exec`, exit status, shell exit reasons, or new shell protocol
  control fields.
- Do not add `--fresh`, `--no-reuse`, or change `tmux new -A -s <session>`
  behavior.
- Do not redesign daemon LocalAPI health, version capability checks, stale
  binary handling, dynamic resize automation, or output hash validation.
- Do not broaden punching, dataplane, or GUI behavior.

## Decisions

### Parse default tmux output only where needed

Windows `wsl:<distro>` session listing will keep avoiding
`tmux list-sessions -F "#S"` because that argument path was observed to be
fragile through `wsl.exe`. Instead, it will parse the default
`tmux list-sessions` output by extracting the session name before the first
colon.

Other paths that can safely use `-F "#S"` should keep doing so because that is
the most direct session-name output.

Alternative considered: force all targets to use default tmux output. That
would unnecessarily widen the parsing surface and risk changing Linux/SSH paths
that already work with `-F "#S"`.

### Build Windows SSH commands without remote `--`

Windows `ssh:<host>` session list and tmux preflight will construct commands in
the same style as attach: `ssh <host> tmux ...`, with flags before the host when
needed. A `--` token after the host is treated as part of the remote command by
OpenSSH, so it must not be used as a local option terminator there.

Alternative considered: keep the current command shape because tests do not use
a live SSH host. This leaves real SSH targets dependent on remote shell behavior
for a token that attach does not send.

### Readiness is bridge setup plus immediate failure polling

`serveShAttach` will send attach-ready after the backend has been created and
the bridge goroutines/channels are installed, while checking for any already
available runtime failure or backend wait result without sleeping. Once ready is
sent, existing runtime handling remains responsible for final success or
failure, including `shell_exit ok=true` after a nil backend wait.

Alternative considered: add a new PTY readiness signal to the `shelltarget.PTY`
interface. Current Unix PTY and ConPTY implementations do not expose such a
signal, and adding one would exceed the release-hardening scope.

### Separate invite emission from IP canonicalization

Invite-code broker emission will continue to normalize, validate, de-duplicate,
probe reachability, and emit the original reachable hostname endpoint. Any
helper that resolves hostnames to IPs must be named or documented as not for
invite-code emission.

Alternative considered: remove the IP-canonicalizing helper. It may still be
useful for non-invite diagnostics or deterministic connect address experiments,
so the safer release hardening is to make the boundary explicit and test the
invite path.

## Risks / Trade-offs

- Default tmux output parsing assumes session names are separated from metadata
  by the first colon. This matches tmux default output and is scoped to the WSL
  compatibility path.
- Immediate failure polling cannot prove a backend is fully interactive before
  sending ready. It does remove the arbitrary sleep and preserves existing
  post-ready failure handling without widening the PTY interface.
- Renaming or documenting helper boundaries can require small test updates in
  `internal/controlplane`, but invite-code wire behavior remains compatible.
