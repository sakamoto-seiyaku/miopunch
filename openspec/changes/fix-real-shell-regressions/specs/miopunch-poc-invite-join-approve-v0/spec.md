## ADDED Requirements

### Requirement: invite preserves reachable hostname broker endpoints
The system SHALL preserve selected reachable hostname broker endpoints in the emitted invite code.

The invite task SHALL still normalize, validate, de-duplicate, and probe
selected endpoints for reachability before emitting the invite code.

#### Scenario: Reachable hostname broker is emitted as hostname
- **WHEN** an invite broker candidate is provided as `host:port`
- **AND** the endpoint passes broker reachability probing
- **THEN** the invite code contains the same normalized `host:port` endpoint
- **AND** the invite code does not replace the hostname with a resolved IP
  address

#### Scenario: Unreachable hostname broker is not emitted
- **WHEN** an invite broker candidate is provided as `host:port`
- **AND** the endpoint fails broker reachability probing
- **THEN** the invite task does not emit that endpoint in `invite_brokers`
- **AND** task diagnostics identify the skipped broker endpoint
