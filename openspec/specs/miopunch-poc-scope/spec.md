# miopunch-poc-scope Specification

## Purpose
`miopunch-poc-scope` defines what the repository considers "POC done" and the explicit constraints for that POC acceptance boundary.
It also defines the required failure explainability contract for POC user flows.

## Requirements
### Requirement: POC acceptance boundary is join → ping → sh(tmux)
The system SHALL define the POC acceptance boundary as a successful completion of `join → ping → sh(tmux)` without requiring a centralized data-plane relay. The system SHALL document the required preconditions and the supported/unsupported network constraints for this boundary.

#### Scenario: Successful POC vertical slice under required preconditions
- **WHEN** a user runs the POC flow (`join → ping → sh(tmux)`) under the documented required preconditions (e.g. control-plane connectivity is available)
- **THEN** the flow completes successfully
- **AND** the user is not required to enable or configure any centralized data-plane relay service

### Requirement: POC failures are explainable with stage, reason_code, facts, and suggestions
The system SHALL provide actionable failure output for POC commands. On any failure, the output SHALL include `stage`, `reason_code`, `facts`, and `suggestions` so users can understand what failed and what to do next.

#### Scenario: A POC command fails and returns actionable failure output
- **WHEN** a POC command fails
- **THEN** the output includes `stage`, `reason_code`, `facts`, and `suggestions`
- **AND** `suggestions` contain at least one concrete user action (e.g. retry, check clock, change broker, change seed)

### Requirement: POC does not fallback to data-plane relay
The system SHALL NOT fallback to a centralized data-plane relay when establishing a P2P data-plane fails. The system MUST fail fast with a clear explanation and remediation suggestions.

#### Scenario: Data-plane cannot be established without relay
- **WHEN** a POC shell session cannot establish a P2P data-plane path
- **THEN** the command fails with actionable output
- **AND** the system does not use any centralized data-plane relay fallback
