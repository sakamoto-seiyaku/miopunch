## ADDED Requirements

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
