## ADDED Requirements

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
