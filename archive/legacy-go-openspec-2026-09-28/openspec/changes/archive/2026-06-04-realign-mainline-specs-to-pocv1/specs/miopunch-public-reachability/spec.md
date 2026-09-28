## MODIFIED Requirements

### Requirement: Internal STUN Defaults For Current POC v1
When the user does not explicitly configure STUN servers, the system SHALL use the current POC v1 internal default STUN endpoint list.

The current POC v1 internal list SHALL be treated as one ordinary best-effort STUN source set for UDP mapped address discovery.

The system SHALL NOT require cn/global bucket arbitration for current POC v1 path establishment.

#### Scenario: Explicit STUN disables internal STUN
- **WHEN** the user explicitly configures STUN servers through CLI or YAML
- **THEN** the system uses only the user-provided STUN servers
- **AND** the system does not use internal STUN defaults

#### Scenario: Internal STUN has no cn/global current gate
- **WHEN** the user does not configure any STUN servers
- **THEN** current POC v1 may sample the internal STUN endpoint list
- **AND** current POC v1 does not require cn/global selected-view arbitration evidence

### Requirement: Observability Of STUN Discovery
The system SHALL record STUN discovery outcomes needed to diagnose current POC v1 UDP path establishment.

At `debug` log level, the system SHALL record which configured or internal STUN endpoints were attempted and whether usable mapped addresses were gathered.

#### Scenario: Debug logs include STUN attempt results
- **WHEN** log level is `debug`
- **THEN** logs include STUN endpoint attempt results and mapped address availability
- **AND** logs do not require cn/global arbitration reasons for current POC v1
