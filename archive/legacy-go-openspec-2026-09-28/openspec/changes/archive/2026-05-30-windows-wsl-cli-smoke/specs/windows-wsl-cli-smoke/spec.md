## ADDED Requirements

### Requirement: Windows and WSL CLI smoke covers bidirectional join

The system SHALL provide a CLI-only smoke path that validates Windows and WSL can each create a network and complete join from the other side.

#### Scenario: Windows creates, WSL joins
- **WHEN** a Windows session bundle runs `up`, `init-network`, `invite`, `approve`, and `join` through the CLI
- **THEN** the WSL side can complete the corresponding join path with the same invite code
- **AND** the run captures the CLI outputs and diagnostics artifacts for both sides

#### Scenario: WSL creates, Windows joins
- **WHEN** a WSL session bundle runs `up`, `init-network`, `invite`, `approve`, and `join` through the CLI
- **THEN** the Windows side can complete the corresponding join path with the same invite code
- **AND** the run captures the CLI outputs and diagnostics artifacts for both sides

### Requirement: CLI smoke must preserve failure evidence

The system SHALL preserve structured failure evidence for each smoke step, including exit code, stage, reason code, facts, and suggestions.

#### Scenario: Join failure is diagnosable
- **WHEN** `join` fails on either Windows or WSL
- **THEN** the recorded evidence includes the failure stage, `reason_code`, `facts`, and `suggestions`
- **AND** the run stores the corresponding CLI stderr and report output

### Requirement: CLI smoke must isolate run state and logs

The system SHALL run the Windows and WSL CLI smoke cases in isolated extracted bundle directories with separate state and logs.

#### Scenario: Independent bundle roots
- **WHEN** the smoke is executed on both Windows and WSL
- **THEN** each side uses its own extracted bundle root, state path, and log files
- **AND** the run metadata records the exact directories used for each side

### Requirement: CLI smoke must require explicit diagnostics capture

The system SHALL collect stdout, stderr, report output, daemon logs, and runtime state snapshots for each smoke run.

#### Scenario: Smoke artifact set is complete
- **WHEN** a smoke case finishes
- **THEN** the artifacts include CLI stdout, CLI stderr, report output, daemon logs, and a runtime or state snapshot when available
- **AND** the artifacts are kept under a run-specific directory
