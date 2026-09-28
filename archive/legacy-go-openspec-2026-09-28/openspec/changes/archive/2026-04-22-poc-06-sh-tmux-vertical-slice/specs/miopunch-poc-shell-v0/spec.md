## ADDED Requirements

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
When attaching to `GET /api/v0/tasks/<task_id>/ws` for a `sh_attach` task, the
client SHALL negotiate `Sec-WebSocket-Protocol: miopunch.sh.v0`.

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
