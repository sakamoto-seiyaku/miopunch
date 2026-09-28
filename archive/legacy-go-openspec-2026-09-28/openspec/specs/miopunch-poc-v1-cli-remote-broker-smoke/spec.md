# miopunch-poc-v1-cli-remote-broker-smoke Specification

## Purpose
Define a cheap Linux-only smoke gate for the current POC v1 product CLI path using one VM, two Docker node containers, and a shared remote MQTT broker.

## Requirements

### Requirement: The CLI pre-gate SHALL use one VM and two Docker node containers
The system SHALL provide a Linux-only smoke gate that runs in one VM and starts exactly two Docker node containers as the test nodes.

The gate SHALL NOT require two VMs and SHALL NOT require a VM-local MQTT broker.

#### Scenario: Smoke topology is bounded
- **WHEN** the CLI pre-gate is started
- **THEN** it provisions one guest VM test environment
- **AND** it starts two Docker node containers inside that VM
- **AND** it does not start a third node or broker container as part of the default smoke topology

### Requirement: The CLI pre-gate SHALL require a host-supplied remote broker URL
The smoke gate SHALL require the broker URL to be supplied explicitly from the host environment.

If the broker URL is missing, the smoke gate MUST fail fast before starting the CLI workflow.

#### Scenario: Missing broker URL fails fast
- **WHEN** the smoke gate is invoked without `MIOPUNCH_POC_V1_CLI_SMOKE_BROKER_URL`
- **THEN** the invocation fails
- **AND** the failure explains that the broker URL must be provided explicitly

### Requirement: The CLI pre-gate SHALL cover only the positive-path product workflow
The smoke gate SHALL validate exactly this product path:
`up -> init-network -> invite -> approve -> join -> ls -> ping -> sh ls`.

The smoke gate SHALL NOT require `sh attach` or revoke validation.

#### Scenario: Positive-path smoke stops at sh ls
- **WHEN** the smoke gate completes successfully
- **THEN** it has exercised `up`, `init-network`, `invite`, `approve`, `join`, `ls`, `ping`, and `sh ls`
- **AND** it has not required `sh attach`
- **AND** it has not required `revoke`

### Requirement: The CLI pre-gate SHALL use the product CLI daemon path
Each node container SHALL start the product daemon through `miopunch up` with an explicit localapi address, state path, and broker override.

The smoke gate SHALL drive the rest of the workflow through the normal product CLI commands against that daemon.

#### Scenario: Smoke uses product CLI commands end to end
- **WHEN** the smoke gate starts both node containers
- **THEN** each node daemon is started with `miopunch up`
- **AND** follow-up actions use the product CLI over localapi rather than direct test-only runtime helpers

### Requirement: The CLI pre-gate SHALL retain artifacts for diagnosis
The smoke gate SHALL collect per-node diagnostic artifacts, including daemon logs and runtime state snapshots, whenever they are available.

#### Scenario: Smoke stores node diagnostics
- **WHEN** the smoke gate exits
- **THEN** artifacts include per-node daemon logs when they exist
- **AND** artifacts include per-node runtime state snapshots when they exist
