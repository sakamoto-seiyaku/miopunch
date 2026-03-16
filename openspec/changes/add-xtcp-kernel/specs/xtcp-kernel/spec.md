## ADDED Requirements

### Requirement: Coordinator-Assisted UDP Traversal Kernel
The system SHALL provide a coordinator-assisted UDP traversal workflow that can negotiate a P2P session between two peers behind NAT.

#### Scenario: Establish a session in a representative easy NAT case
- **GIVEN** a NAT lab environment is available with a representative `NAT1 x NAT1` case (e.g., `core-01`)
- **WHEN** two peers run the `xtcp-kernel` CLI against a coordinator to establish a session
- **THEN** the peers establish a P2P UDP session
- **AND** the peers can exchange application payload over that session

### Requirement: Stage-Level Observability
The system SHALL expose a stage-based, machine-readable timeline for each traversal attempt.

#### Scenario: Explain a failure by stage
- **GIVEN** a traversal attempt fails in a NAT lab case
- **WHEN** the developer inspects the run output and artifacts
- **THEN** the failure is attributed to a specific stage
- **AND** the system records relevant conditions (candidates, retries, timeouts, and decisions) needed for diagnosis

### Requirement: Explicit and Observable Fallback
The system SHALL provide a controlled fallback mode that is explicit, observable, and testable.

#### Scenario: Fallback is visible and does not hide the primary failure
- **GIVEN** a traversal attempt cannot establish a direct P2P session
- **WHEN** the system engages a fallback mode
- **THEN** the fallback decision is reported explicitly
- **AND** the original failure stage and reason remain visible to the user

### Requirement: NAT Lab Regression Entry Point
The system SHALL provide a repeatable integration test entry point that runs against the `P0` NAT lab testbed.

#### Scenario: Run representative lab cases
- **GIVEN** the `P0` NAT lab testbed is available
- **WHEN** the integration regression suite is executed
- **THEN** representative success and failure paths are exercised
- **AND** artifacts required for diagnosis are collected on failure

### Requirement: Control Plane Transport Selection
The system SHALL support selecting the `control plane` transport protocol among `TCP`, `KCP`, and `QUIC`.

#### Scenario: Connect to coordinator using a selected protocol
- **GIVEN** the developer configures the `xtcp-kernel` CLI to use a specific `control plane` transport protocol
- **WHEN** the peer connects to the coordinator
- **THEN** the connection is established using the selected protocol
- **AND** the selection is visible in machine-readable diagnostics
