## ADDED Requirements

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
