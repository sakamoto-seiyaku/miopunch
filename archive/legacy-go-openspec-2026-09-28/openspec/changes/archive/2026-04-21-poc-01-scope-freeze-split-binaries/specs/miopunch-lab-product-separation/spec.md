## ADDED Requirements

### Requirement: Lab and product entrypoints are separate binaries
The system SHALL provide separate binaries for lab/experiments and product/POC usage:

- `miopunch-lab`: lab/experiments entrypoint
- `miopunch`: product/POC entrypoint

The system SHALL NOT expose lab subcommands (`coord/peer/stun/mqtt-broker`) from the product binary.

#### Scenario: Lab commands are available from miopunch-lab
- **WHEN** a user runs `miopunch-lab coord --help`
- **THEN** the command succeeds and shows usage for the coordinator

#### Scenario: Lab commands are not available from miopunch
- **WHEN** a user runs `miopunch coord`
- **THEN** the command fails
- **AND** the output instructs the user to use `miopunch-lab coord` instead

### Requirement: Lab automation builds and runs miopunch-lab
The lab automation scripts in this repository SHALL build and run the lab binary from `./cmd/miopunch-lab` and SHALL NOT depend on the product binary for lab execution.

#### Scenario: labctl builds the lab binary from cmd/miopunch-lab
- **WHEN** a developer runs `./lab/host/labctl push-bin`
- **THEN** the built binary corresponds to `./cmd/miopunch-lab`
- **AND** the guest execution path uses the `miopunch-lab` binary
