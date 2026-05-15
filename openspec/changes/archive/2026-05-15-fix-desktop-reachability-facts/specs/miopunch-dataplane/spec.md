## ADDED Requirements

### Requirement: Peer Session Summaries Preserve Safe Endpoint Facts
Peer session summaries SHALL preserve safe local and remote endpoint facts when the selected transport session can report them.

The summary MUST NOT include secret keys, raw candidate lists, invite material, or unvalidated public tuple values.

#### Scenario: TLS session summary includes elected TCP endpoints
- **WHEN** a TLS peer session is established over an elected TCP connection
- **THEN** its session summary includes the selected local and remote endpoints
- **AND** the summary does not include non-selected candidate endpoints

#### Scenario: UDP session summary includes selected UDP endpoints
- **WHEN** a UDP-backed peer session is established with a selected remote UDP endpoint
- **THEN** its session summary includes the selected local and remote endpoints

### Requirement: Session Wrappers Preserve Path Facts
Session wrappers that own additional resources around a peer session SHALL preserve any path facts reported by the wrapped session.

#### Scenario: Owned session forwards path facts
- **WHEN** a session wrapper is listed through the session manager
- **THEN** the resulting summary includes the wrapped session path facts when available
