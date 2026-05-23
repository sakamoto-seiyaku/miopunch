# miopunch-poc-v1-gui-wizard Specification

## Purpose
Defines the POC v1 GUI wizard stages and the user-facing output contract.

## ADDED Requirements

### Requirement: GUI stages are fixed to 6
The system SHALL implement exactly 6 GUI stages: Network, Enroll, Discover, Punch, SecureSession, Shell.

#### Scenario: Wizard progresses through the fixed six-stage flow
- **WHEN** a user runs the v1 desktop flow from network setup to shell
- **THEN** the GUI presents exactly the stages Network, Enroll, Discover, Punch, SecureSession, and Shell
- **AND** it does not insert extra half-stages into the default path

### Requirement: Summary + Evidence output contract
The system SHALL present a short UserSummary (<=3 lines) per stage and SHALL provide an Evidence view for detailed diagnostics.

#### Scenario: Stage output separates user guidance from diagnostics
- **WHEN** a stage completes or fails in the v1 desktop flow
- **THEN** the user sees a short summary of at most 3 lines
- **AND** detailed diagnostics are available separately through Evidence

### Requirement: reason_code cardinality is bounded
The system SHALL bound reason_code to at most 12 values; adding a new reason_code requires merging/replacing an existing one.

#### Scenario: Reason code growth is explicitly constrained
- **WHEN** a new failure category is proposed for the v1 GUI
- **THEN** the total `reason_code` set remains at or below 12 values
- **AND** the new category replaces or merges with an existing code if the budget is full
