## ADDED Requirements

### Requirement: STUN Endpoint Scheme Prefixes
The system SHALL accept STUN endpoints in the following forms:
- `host:port`: a dual endpoint that MAY be used for both UDP and TCP STUN
- `udp://host:port`: an endpoint restricted to UDP STUN
- `tcp://host:port`: an endpoint restricted to TCP STUN

When the system performs UDP STUN sampling, it SHALL ignore `tcp://` endpoints.
When the system performs TCP STUN sampling, it SHALL ignore `udp://` endpoints.

#### Scenario: UDP sampling ignores TCP-only endpoints
- **WHEN** a STUN endpoint list contains both `tcp://host:port` and a UDP-compatible endpoint
- **AND** the system performs UDP STUN sampling
- **THEN** the `tcp://` endpoint does not cause the run to fail
- **AND** only UDP-compatible endpoints are used for UDP STUN sampling

#### Scenario: Explicit STUN config fails fast if no usable endpoints remain
- **WHEN** the user explicitly configures STUN endpoints
- **AND** after applying the UDP/TCP scheme filter, no endpoints remain usable for the configured STUN sampling protocol
- **THEN** the system fails with a configuration error
