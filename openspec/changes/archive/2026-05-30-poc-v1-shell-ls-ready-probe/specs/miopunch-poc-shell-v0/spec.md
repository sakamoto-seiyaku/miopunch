## MODIFIED Requirements

### Requirement: sh_ls lists discoverable targets, ready targets, and tmux sessions
The system SHALL implement `sh_ls` as a read-only task:
- When target is omitted and ready-probe mode is not requested, the result
  SHALL include all discoverable targets for the peer.
- When target is omitted and ready-probe mode is requested, the system SHALL
  probe each discoverable target for tmux-backed attach readiness and the
  result SHALL include only targets confirmed ready.
- When a target is specified, the result SHALL include available tmux sessions
  for that target.
- Ready-probe mode SHALL apply only to peer-level target listing and SHALL NOT
  be combined with target-scoped session listing.

#### Scenario: sh_ls without target lists discoverable targets
- **WHEN** a user runs `miopunch sh ls <peer>`
- **THEN** the result lists discoverable targets for that peer

#### Scenario: sh_ls with target lists tmux sessions
- **WHEN** a user runs `miopunch sh ls <peer> <target>`
- **THEN** the result lists tmux sessions available on that target

#### Scenario: sh_ls ready probe lists only ready targets
- **WHEN** a user runs `miopunch sh ls <peer> --ready`
- **THEN** the result lists only targets confirmed ready for tmux-backed attach
- **AND** targets that are not confirmed ready are excluded from the line-level
  ready list

#### Scenario: sh_ls rejects ready probe with target-scoped session listing
- **WHEN** a user requests ready-probe mode together with a concrete
  `<target>`
- **THEN** the command fails as a bad request
- **AND** the output instructs the user to choose either peer-level ready
  listing or target-scoped session listing

## ADDED Requirements

### Requirement: sh_ls ready probing is bounded and classifies every discovered target
When `sh_ls` runs in ready-probe mode, the system SHALL classify each
discovered target as exactly one of:
- `ready`
- `unsupported`
- `unknown`

`ready` SHALL mean the system confirmed that tmux attach preconditions are
available on the target without requiring an already-running tmux session.

`unsupported` SHALL mean the system confirmed that tmux is missing on the
target.

`unknown` SHALL mean the system could not safely confirm readiness within the
bounded non-interactive probe, including timeout, authentication refusal,
host-key refusal, or other probe failure.

The command SHALL succeed when target discovery succeeds even if one or more
targets are classified as `unsupported` or `unknown`.

The operator-visible ready list SHALL contain only `ready` targets, and the
structured success output SHALL preserve per-target classifications for all
discovered targets.

#### Scenario: tmux missing becomes unsupported during ready probe
- **WHEN** a user runs `miopunch sh ls <peer> --ready`
- **AND** one discovered target does not have tmux available
- **THEN** that target is classified as `unsupported`
- **AND** the command still returns success for the overall ready-probe request

#### Scenario: tmux installed without server still counts as ready
- **WHEN** a user runs `miopunch sh ls <peer> --ready`
- **AND** one discovered target can execute tmux but has no running tmux server
- **THEN** that target is classified as `ready`
- **AND** the ready classification does not depend on pre-existing tmux
  sessions

#### Scenario: non-interactive SSH probe failure becomes unknown
- **WHEN** a user runs `miopunch sh ls <peer> --ready`
- **AND** one discovered `ssh:<host>` target cannot be confirmed within the
  bounded non-interactive probe
- **THEN** that target is classified as `unknown`
- **AND** the overall command still returns any other confirmed ready targets
