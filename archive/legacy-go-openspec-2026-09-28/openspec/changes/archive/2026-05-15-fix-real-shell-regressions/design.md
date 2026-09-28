## Context

The real Windows/WSL2 shell run proved that the punching and TLS payload path
can be healthy while higher shell layers still fail from platform-specific text
formats, tmux lifecycle details, or missing shell completion semantics.

The same run also found that invite broker reachability probing can emit a
resolved IP instead of the configured hostname, causing Windows join to fail
even though the hostname broker is reachable.

## Goals / Non-Goals

**Goals:**

- Make the real shell fixes production behavior instead of debug-only patches.
- Preserve stage-specific diagnostics so target dependency failures do not look
  like connectivity failures.
- Keep shell completion local to the shell protocol and bridge, without changing
  punching or dataplane semantics.
- Add focused tests for every observed regression.

**Non-Goals:**

- Do not add dynamic SIGWINCH automation or byte-for-byte output hash checking.
- Do not redesign shell target discovery beyond the observed Windows/WSL2/tmux
  compatibility fixes.
- Do not introduce new daemon-close reason codes in this change.
- Do not treat old daemon processes or stale extracted binaries as product
  behavior changes.

## Decisions

### Preserve invite broker hostnames after reachability probing

Invite broker selection will normalize and validate `host:port` endpoints, probe
the same endpoint for MQTT reachability, and emit that original normalized
endpoint. It will not resolve a hostname and write an A record address into the
invite code.

Alternative considered: keep canonicalizing to IP. Real Windows join showed that
this can make an otherwise reachable hostname broker unreachable from the
joiner, with no fallback available because join must use invite code brokers.

### Decode Windows command output at the shelltarget boundary

Windows shell target discovery will decode command output as UTF-16LE when the
byte pattern indicates UTF-16LE, otherwise it will keep UTF-8/plain output as-is.
This keeps the platform-specific behavior inside `shelltarget` and avoids
leaking NUL-polluted WSL distro names into task results.

### Classify tmux dependency and no-server states separately

Missing `tmux` text from shell, zsh, or Windows cmd will map to
`ErrTmuxMissing` and ultimately `SH_TMUX_MISSING`. Installed tmux with no
running server will return an empty session list. This preserves the difference
between a target dependency problem and a valid target with no sessions.

For Windows `wsl:<distro>` session listing, avoid relying on the fragile
`tmux list-sessions -F "#S"` argument path observed through `wsl.exe`. Parse the
default tmux output for now because it is sufficient for session discovery and
avoids the observed format-argument loss.

### Signal normal shell completion explicitly

The controlled side will send a `shell_exit` control frame with `ok=true` after
the backend PTY/ConPTY wait returns nil. The initiating bridge treats that frame
as successful completion and restores the local terminal. If the backend exits
before attach-ready, the task remains a setup failure.

Alternative considered: infer success from stream close. The real `ssh:ale`
case showed that normal remote exit could leave the initiator waiting until the
local CLI was killed and then misreport as a LocalAPI WebSocket abnormal close.

### Let backend wait decide expected PTY close outcomes

Unix PTY reads can return EOF, closed pipe, or EIO after the child shell exits.
The read loop will treat those as expected close signals and let the backend
wait result decide success or failure. Unexpected read errors and non-nil wait
errors still produce shell-layer diagnostics.

## Risks / Trade-offs

- Default `tmux list-sessions` output may include formatting beyond the session
  name on some tmux versions. The current behavior is acceptable for this
  POC discovery path and is covered by real WSL2 validation.
- UTF-16LE detection is heuristic. Keep it narrow: decode only when high bytes
  strongly indicate UTF-16LE, otherwise preserve plain output.
- `shell_exit` adds a protocol control op. Existing clients that ignore unknown
  text control frames remain unaffected, while the CLI gains reliable success
  completion.
- PTY EIO is treated as expected only in the read-close path; backend wait
  failures still surface as failures.
