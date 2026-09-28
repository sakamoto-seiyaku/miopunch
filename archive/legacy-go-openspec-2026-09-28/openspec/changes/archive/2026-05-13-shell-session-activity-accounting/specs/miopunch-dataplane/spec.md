## ADDED Requirements

### Requirement: Logical stream traffic refreshes peer session activity
The dataplane SHALL treat successful logical-stream traffic as peer-session
activity.

Any logical stream read or write that transfers one or more bytes SHALL refresh
the owning peer session's activity timestamp, regardless of whether the logical
stream carries application data or control traffic.

#### Scenario: Active logical stream stays healthy past idle timeout
- **GIVEN** a healthy peer transport session with an open logical stream
- **AND** the peers continue exchanging bytes on that logical stream at
  intervals shorter than the configured idle timeout
- **WHEN** the overall session lifetime exceeds the idle timeout window
- **THEN** the peer transport session remains healthy
- **AND** it is not closed for idle timeout while that traffic continues

#### Scenario: Truly idle session still closes
- **GIVEN** a healthy peer transport session
- **AND** no logical-stream read, write, open, or close activity occurs for
  longer than the configured idle timeout
- **WHEN** the idle timeout window elapses
- **THEN** diagnostics identify the close reason as idle timeout
- **AND** the next operation must establish a fresh peer session
