# miopunch-poc-shell-v0 Specification

## Purpose
`miopunch-poc-shell-v0` defines the POC v0 shell vertical slice: remote shell
tasks (`sh_ls`, `sh_attach`), target naming, tmux session anchoring semantics,
single-writer lock rules, LocalAPI WebSocket framing (`miopunch.sh.v0`), and
stable reason codes for failures.
## Requirements
### Requirement: Shell targets include Windows WSL/SSH and Linux local
The system SHALL support the following shell target types for `sh_ls` and `sh_attach`:
- Windows controlled node: `wsl:<distro>` and `ssh:<name>`
- Linux controlled node: `local`

#### Scenario: Linux controlled node exposes the local target
- **WHEN** a user lists shell targets for a Linux controlled peer
- **THEN** the result includes a target named `local`

#### Scenario: Windows controlled node exposes WSL and SSH targets
- **WHEN** a user lists shell targets for a Windows controlled peer
- **THEN** the result includes at least one `wsl:<distro>` target when WSL is available
- **AND** the result includes `ssh:<name>` targets for configured SSH shortcuts

### Requirement: sh_attach anchors the session to tmux new -A -s <session>
For a `sh_attach` session, the system SHALL anchor the "现场" to a tmux session by executing:
`exec tmux new -A -s <session>`.

#### Scenario: attach creates or reattaches the tmux session
- **WHEN** a user runs `miopunch sh <peer> [target] -s <session>`
- **THEN** the system creates the tmux session if missing, or attaches if it already exists
- **AND** the user can detach and re-run the command to recover the session

### Requirement: sh_attach enforces a single-writer lock per (peer,target,session)
The system SHALL enforce a single-writer policy for `sh_attach`:
only one active attach is allowed for a given `(peer,target,session)` at a time.

The lock SHALL be kept alive by WebSocket and/or data-plane activity, and SHALL
be released when the session ends. If there is no activity for longer than the
configured TTL, the lock SHALL expire automatically.

#### Scenario: second attach is rejected as in use
- **WHEN** a user attempts `sh_attach` while the same `(peer,target,session)` is already attached
- **THEN** the request fails with `reason_code=SH_IN_USE`
- **AND** the output includes actionable suggestions (e.g., retry later or use a different session)

#### Scenario: lock expires after TTL without activity
- **WHEN** a `sh_attach` session becomes inactive for longer than the configured TTL
- **THEN** the lock is released automatically
- **AND** a new attach attempt for the same `(peer,target,session)` can succeed

### Requirement: LocalAPI WebSocket for sh_attach uses miopunch.sh.v0 frame semantics
The system SHALL require clients attaching to `GET /api/v0/tasks/<task_id>/ws` for a `sh_attach` task to negotiate `Sec-WebSocket-Protocol: miopunch.sh.v0`.

The WebSocket frames SHALL follow:
- binary frames: PTY/ConPTY raw byte stream (stdin/stdout)
- text frames: JSON control messages (minimum: `winsize{cols,rows}`)

#### Scenario: WebSocket carries PTY bytes and resize control
- **WHEN** a client attaches to a `sh_attach` task WebSocket and sends PTY input bytes
- **THEN** the server forwards them to the remote PTY session
- **AND** window resize is delivered via a JSON control message on text frames

### Requirement: sh_ls lists targets and tmux sessions
The system SHALL implement `sh_ls` as a read-only task:
- When target is omitted, the result SHALL include available targets for the peer.
- When a target is specified, the result SHALL include available tmux sessions for that target.

#### Scenario: sh_ls without target lists targets
- **WHEN** a user runs `miopunch sh ls <peer>`
- **THEN** the result lists available targets

#### Scenario: sh_ls with target lists tmux sessions
- **WHEN** a user runs `miopunch sh ls <peer> <target>`
- **THEN** the result lists tmux sessions available on that target

### Requirement: sh failures use stable reason_code identifiers
On failures in `sh_ls` and `sh_attach`, the system SHALL emit stable `reason_code`
identifiers (not renamed within POC v0), including at minimum:
`SH_TARGET_NOT_FOUND`, `SH_TARGET_AMBIGUOUS`, `SH_IN_USE`, `SH_CONNECTOR_FAIL`,
`SH_TMUX_MISSING`, `SH_TMUX_ATTACH_FAIL`.

#### Scenario: missing tmux fails with SH_TMUX_MISSING
- **WHEN** a user attempts `sh_attach` to a target without `tmux` installed or available
- **THEN** the task fails with `reason_code=SH_TMUX_MISSING`
- **AND** the output includes a concrete installation or remediation suggestion

### Requirement: sh_attach interactive CLI exits when remote session ends
The interactive `sh_attach` CLI SHALL restore the local terminal and return when the task WebSocket closes, the remote task ends, or the WebSocket read path fails. It MUST NOT wait for an additional local stdin byte before exiting. After the WebSocket ends, the CLI SHALL still make a bounded best-effort attempt to fetch final task state and export the report when requested.

#### Scenario: Remote close while stdin is idle
- **WHEN** a user is attached with `miopunch sh <peer> ...`
- **AND** the task WebSocket closes while the user is not typing
- **THEN** the CLI restores the terminal and returns without waiting for stdin input
- **AND** final task-state lookup and report export remain bounded

### Requirement: sh_attach late failures preserve shell-layer diagnostics
The system SHALL preserve shell-layer diagnostics when `sh_attach` reaches
interactive attach setup and then ends abnormally, instead of collapsing the
outcome to a generic transport close.

The final task/report diagnostics SHALL include:
- `stage=SessionAttach`
- a stable `reason_code`
- facts identifying the selected peer, target, and session
- facts identifying the failing shell layer when known

The failing shell layer SHALL distinguish the best available boundary, such as
`task_bridge`, `acceptor`, `shelltarget`, `tmux`, `pty`, or `ssh`.

Raw transport evidence such as `EOF`, close code, or backend stderr MAY appear
as supplemental facts, but SHALL NOT be the only operator-facing explanation
when richer shell diagnosis exists.

#### Scenario: Remote shell target failure after attach setup stays attributable
- **WHEN** a `sh_attach` task has already reached interactive attach setup
- **AND** the remote shell target fails before the interactive session becomes
  stable
- **THEN** the final task/report output remains at `stage=SessionAttach`
- **AND** it includes a stable `reason_code`
- **AND** facts identify the selected peer, target, session, and failing shell
  layer

#### Scenario: Remote stream close without richer backend cause still keeps layer context
- **WHEN** a `sh_attach` task closes unexpectedly after attach setup
- **AND** the system cannot recover a richer backend-specific failure cause
- **THEN** the final task/report output still includes `stage=SessionAttach`
- **AND** facts identify the selected peer, target, session, and the last known
  failing shell boundary

### Requirement: Shell target discovery decodes Windows WSL names without NUL pollution
The system SHALL decode Windows command output used for WSL shell target
discovery so that WSL distro names are emitted as clean `wsl:<distro>` target
identifiers.

#### Scenario: Windows WSL list output is UTF-16LE
- **WHEN** a Windows controlled node lists WSL distros and `wsl.exe -l -q`
  returns UTF-16LE text
- **THEN** `sh ls <peer>` emits clean `wsl:<distro>` targets
- **AND** target names do not contain embedded NUL bytes

#### Scenario: Windows WSL list output is already UTF-8
- **WHEN** a Windows controlled node lists WSL distros and command output is
  already plain UTF-8 text
- **THEN** `sh ls <peer>` preserves the distro names without applying UTF-16
  corruption

### Requirement: tmux dependency and empty-server states are classified distinctly
The system SHALL distinguish missing `tmux` from an installed `tmux` that has no
running server when listing or attaching shell sessions.

#### Scenario: tmux is missing on the target
- **WHEN** `sh ls` or `sh_attach` reaches a shell target where `tmux` is not
  installed or cannot be found
- **THEN** the task fails with `reason_code=SH_TMUX_MISSING`
- **AND** diagnostics identify that the target needs `tmux`, not that peer
  connectivity failed

#### Scenario: tmux is installed but no server is running
- **WHEN** `sh ls <peer> <target>` reaches a shell target where `tmux` is
  installed but no tmux server exists
- **THEN** the task succeeds
- **AND** the result contains no session facts for that target

### Requirement: sh_attach reports normal remote shell completion explicitly
The shell attach stream SHALL carry an explicit control message when the remote
PTY/ConPTY backend exits normally after the interactive session is ready.

#### Scenario: Remote shell exits normally after attach is ready
- **WHEN** a user is attached with `miopunch sh <peer> <target> -s <session>`
- **AND** the remote shell exits normally after the attach stream is ready
- **THEN** the controlled side sends a `shell_exit` control message with
  `ok=true`
- **AND** the initiating CLI returns success without waiting for more local
  stdin

#### Scenario: Remote shell exits before attach is ready
- **WHEN** the remote PTY/ConPTY backend exits before the attach stream becomes
  ready
- **THEN** the task fails as a shell setup failure
- **AND** the early exit is not reported as a successful `shell_exit`

### Requirement: Unix PTY expected read close does not mask backend success
The system SHALL treat expected Unix PTY read-close errors after child exit as
close semantics, and SHALL use the backend wait result to decide final success
or failure.

#### Scenario: PTY read returns EIO after child exits normally
- **WHEN** a Linux PTY read returns `/dev/ptmx` input/output error after the
  shell child exits normally
- **THEN** `sh_attach` completes successfully
- **AND** the task is not failed as `SH_CONNECTOR_FAIL`

#### Scenario: Backend wait returns an error
- **WHEN** the PTY/ConPTY backend wait returns a non-nil error
- **THEN** `sh_attach` fails with shell-layer diagnostics
- **AND** the wait failure is not hidden by expected read-close handling

### Requirement: Windows WSL tmux session discovery emits clean session names
The system SHALL emit only tmux session names from `sh ls <peer> wsl:<distro>`
even when the Windows WSL path uses default `tmux list-sessions` output instead
of `tmux list-sessions -F "#S"`.

#### Scenario: Default tmux output is parsed to session names
- **WHEN** a Windows controlled node lists sessions for `wsl:<distro>`
- **AND** `tmux list-sessions` returns default output such as
  `main: 1 windows`
- **THEN** `sh ls <peer> wsl:<distro>` emits `session=main`
- **AND** the session fact does not include tmux metadata after the session name

#### Scenario: Empty tmux server remains an empty session list
- **WHEN** a Windows controlled node lists sessions for `wsl:<distro>`
- **AND** tmux reports that no server is running
- **THEN** the task succeeds
- **AND** no session facts are emitted for that target

### Requirement: Windows SSH tmux commands use attach-compatible remote command shape
The system SHALL construct Windows `ssh:<host>` tmux list and tmux preflight
commands without passing a remote `--` token before `tmux`.

#### Scenario: SSH tmux list command invokes remote tmux directly
- **WHEN** a Windows controlled node lists sessions for `ssh:<host>`
- **THEN** the command invokes SSH with the host followed by `tmux list-sessions`
- **AND** the remote command does not start with `--`

#### Scenario: SSH tmux preflight command invokes remote tmux directly
- **WHEN** a Windows controlled node preflights tmux before attaching to
  `ssh:<host>`
- **THEN** the command invokes SSH with the host followed by `tmux -V`
- **AND** the remote command does not start with `--`

### Requirement: sh_attach readiness is not based on a fixed sleep window
The system SHALL send shell attach ready after backend creation and bridge setup
without relying on a fixed sleep duration as the readiness boundary.

If the backend has already reported a failure or exited before ready can be
sent, the system SHALL report setup failure instead of sending attach-ready.
After attach-ready is sent, backend wait results SHALL continue to decide normal
completion or shell-layer failure.

#### Scenario: Backend failure before ready remains setup failure
- **WHEN** the shell backend reports failure before attach-ready is sent
- **THEN** `sh_attach` fails as a setup failure
- **AND** the controlled side does not send a successful attach-ready control
  message first

#### Scenario: Backend exits normally after ready sends shell_exit
- **WHEN** attach-ready has been sent
- **AND** the shell backend wait result is nil
- **THEN** the controlled side sends `shell_exit` with `ok=true`
- **AND** the initiating side completes the task successfully
