# miopunch-poc-scope Specification

## Purpose
`miopunch-poc-scope` defines what the repository considers "POC done" and the explicit constraints for that POC acceptance boundary.
It also defines the required failure explainability contract for POC user flows.

## Requirements
### Requirement: POC acceptance boundary is GUI-led and includes ping before shell
The system SHALL define the current extracted POC acceptance boundary as a successful completion of the desktop-guided flow `Network → Enroll → Discover → Punch → SecureSession → Shell` without requiring a centralized data-plane relay.

The `SecureSession` stage SHALL include at least one successful identity-bound `ping` or `hello` exchange before the flow is considered ready for `Shell`.

The CLI shorthand `join → ping → sh(tmux)` MAY remain as a diagnostic expression of the same acceptance boundary.

#### Scenario: Successful POC vertical slice under required preconditions
- **WHEN** a user runs the extracted POC flow under the documented required preconditions (e.g. control-plane connectivity is available)
- **THEN** the flow completes successfully
- **AND** the flow reaches `Shell` only after a successful `ping` or `hello` inside `SecureSession`
- **AND** the user is not required to enable or configure any centralized data-plane relay service

### Requirement: POC failures are explainable with stage, reason_code, facts, and suggestions
The system SHALL provide actionable failure output for POC commands. On any failure, the output SHALL include `stage`, `reason_code`, `facts`, and `suggestions` so users can understand what failed and what to do next.
When the extracted v1 GUI or runtime packages these fields as structured `Evidence`, that wrapper SHALL preserve `facts` and `suggestions` without flattening or loss.

#### Scenario: A POC command fails and returns actionable failure output
- **WHEN** a POC command fails
- **THEN** the output includes `stage`, `reason_code`, `facts`, and `suggestions`
- **AND** `suggestions` contain at least one concrete user action (e.g. retry, check clock, change broker, change seed)
- **AND** any structured `Evidence` wrapper preserves `facts` and `suggestions` as separately inspectable fields

### Requirement: POC does not fallback to data-plane relay
The system SHALL NOT fallback to a centralized data-plane relay when establishing a P2P data-plane fails. The system MUST fail fast with a clear explanation and remediation suggestions.

#### Scenario: Data-plane cannot be established without relay
- **WHEN** a POC shell session cannot establish a P2P data-plane path
- **THEN** the command fails with actionable output
- **AND** the system does not use any centralized data-plane relay fallback
