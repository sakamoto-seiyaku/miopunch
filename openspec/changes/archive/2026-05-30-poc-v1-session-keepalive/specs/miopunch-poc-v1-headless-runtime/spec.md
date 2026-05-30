## ADDED Requirements

### Requirement: Validated peer sessions remain reusable across idle gaps
The current v1 runtime SHALL keep a healthy peer session reusable after a
successful identity-bound `ping` or `hello` exchange.

The runtime SHALL send bounded application-level keepalive traffic for such
validated sessions so that a later `sh` can reuse the existing session instead
of forcing a fresh punch after a short idle gap.

#### Scenario: Pinged session remains reusable for later sh
- **GIVEN** a healthy peer session has completed a successful `ping` or
  `hello` exchange
- **AND** no other application traffic occurs for a period shorter than the
  keepalive budget
- **WHEN** a later `sh` targets the same peer
- **THEN** the runtime can reuse the existing healthy session
- **AND** it does not need to establish a fresh session solely because of the
  idle gap

#### Scenario: Truly idle sessions still close
- **GIVEN** a peer session has not been validated by `ping` or `hello`
- **OR** the session has no traffic for longer than the dataplane idle timeout
- **WHEN** the idle timeout elapses
- **THEN** the session is still closed by the dataplane idle closer
- **AND** the next operation must establish a fresh session
