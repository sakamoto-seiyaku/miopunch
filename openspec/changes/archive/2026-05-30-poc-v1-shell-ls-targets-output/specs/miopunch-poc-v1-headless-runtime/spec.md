## ADDED Requirements

### Requirement: sh ls exposes concrete targets and sessions in operator-visible success output
When `miopunch sh ls <peer>` succeeds, the system SHALL expose the concrete target names in operator-visible success output and report output.
When `miopunch sh ls <peer> <target>` succeeds, the system SHALL expose the concrete session names in operator-visible success output and report output.

#### Scenario: sh ls without a target shows the available targets
- **WHEN** a user runs `miopunch sh ls <peer>` against a Windows-controlled peer
- **THEN** the success output includes the concrete `wsl:<distro>` and `ssh:<name>` targets
- **AND** the existing line output remains unchanged

#### Scenario: sh ls with a target shows the available sessions
- **WHEN** a user runs `miopunch sh ls <peer> <target>`
- **THEN** the success output includes the concrete tmux session names for that target
- **AND** the existing line output remains unchanged
