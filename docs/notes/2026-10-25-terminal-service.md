# Terminal service integration

Engineering choices for MIOPUNCH-120, implementing M107 stories 55–58 and
ADRs 0017/0018. These are module choices within the accepted scope; no formal
terminal interface or additional permission model is introduced.

The provider uses tmux's real pane PTY and control-mode attachment. A retained
workspace is a named tmux session. An attachment is a separate control-mode
client and admitted network connection. Detaching, losing a connection, closing
a client, disabling the service, or losing permission closes attachments while
retaining workspaces. Only the explicit end operation kills a selected session.
Host/tmux termination does not promise process recovery.

Linux launches tmux directly. Windows launches the same tmux operations through
`wsl.exe --distribution <selected distro> --exec`, so the terminal is in that
WSL Linux environment. Browser and Android only consume terminals. The local
provider configuration specifies an owned private directory, shell and optional
WSL distribution; no default/user tmux server or shell history is consulted.
Fresh sessions start `/bin/bash --noprofile --norc` with explicit arguments
rather than a shell-assembled remote command. Their initial directory is a
fresh workspace beneath the host-configured provider directory. The shell
still operates in the real host/selected WSL environment and can use workspace
paths that the host account permits. No product filesystem allowlist or fresh
workspace jail is introduced. Terminal permission does not grant a separate
network-management or credential API capability; those APIs continue their
existing checks. An intentionally shared shell has the filesystem rights of
the provider's host account, so that is not a credential-isolation promise.
Tests use only fresh owned paths/commands and the qualified whole-scenario
loopback/PID/mount namespace; those safeguards do not narrow product behavior.

The dedicated terminal ALPN uses the existing connection/admission foundation
and the one signed service policy. Service grants apply to terminal operations
on that provider, not node management. Native and WASM share the bounded wire
records and user operations. Every new operation and ongoing input/output
requires current service authorization; periodic re-evaluation preserves the
existing original guest deadline and immediate received-policy enforcement.
Workspace names do not confer permission. Native tracks a bounded set of
attachment owners; callers consume bounded chunks rather than putting terminal
history into node status. Queue saturation must not block cancellation or detach.

Configuration is an explicit local operation. It never installs WSL, tmux or a
shell, changes host configuration, or enables service merely because a
capability is present. Capability, enabled policy, current access, connection,
active attachments and failure remain separate observations. Runtime capability
probes only inspect the explicitly configured environment.

Actual Windows qualification remains a requirement of M120. Local Linux,
cross-compilation, prepared actions and historical WSL discovery cannot satisfy
it. The Windows WSL provider validation is explicitly deferred by the user; the
current Windows Native consumer qualification below does not cover that role.


## Current qualification and bounded Browser sends — 2026-10-29

[Current platform evidence](../evidence/terminal-2026-10-29/current-platform-qualification.json)
qualifies 17 real scenarios against the current Native graphs and repaired WASM
package. Linux provides the actual PTYs. Windows consumes two concurrent
workspaces through SDK connections, with independent Linux file observations,
controlled work continuing after detach, reattachment, selected end and live
access shutdown. Browser tests cover real tab closure, saved identity reopening,
ACL changes, guest scopes and original expiry, input/output limits and cleanup.
Windows WSL provision remains deferred; M120 is not fully accepted.

The Browser input stress check exposed unbounded work before the bounded channel:
1,024 concurrent sends opened storage reads before any input was admitted. The
actual event loop then missed timer service for 36.8 seconds and detach took
40.7 seconds. Each attachment now serializes send preparation, retains storage
and authorization checks for every message, races the whole operation against
cancellation, and yields a browser task turn between accepted records. This is
an implementation correction to the existing bounded-cancellation contract.
The identical burst and a separate accepted-prefix scenario now settle in about
5.04 seconds. A deadline error remains an error; it does not mean normal detach,
delivery of every queued input, or termination of the retained workspace.

The current SDK checks also qualify pending-future cancellation, explicit close,
Drop/reopen, 16 Native attachments, eight Browser attachments, 16 workspaces,
16 KiB input, oversized rejection and finite 16 MiB unread output. Assertions
check real effects and owner settlement. Original failures, their source
snapshots and the later results remain separate. One Chrome startup fixture
failure came from requiring two identical argv observations despite Chromium
rewriting argv. The host now keeps the first exact original-child observation
and uses normal protocol close with the same start identity; it adds no OS
signal authority. The original failed case required later namespace-settlement
verification and is retained as failed.

No Native SDK or dependency rebuild was needed for the Browser correction. The
WASM build reused 295 artifacts and rebuilt one. The raw evidence initially
exceeded its 16 MiB estimate because repeated Playwright source snapshots were
omitted from that estimate. The review preserves this overrun, sets a 32 MiB
completion budget, and reuses verified snapshots prospectively. The retained
raw evidence is 23,147,714 bytes; SDK pins use the existing shared store. Runtime
private-directory peaks and blocks actually freed in the failed-profile cleanup
were not measured.
