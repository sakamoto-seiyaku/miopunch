## ADDED Requirements

### Requirement: Derived Transport Regression Under Loss
The system SHALL provide a derived high-loss lab regression on a representative easy NAT baseline to verify the current `KCP` and `QUIC` data plane still exchanges application payload after traversal succeeds.

#### Scenario: Validate KCP payload exchange under high loss
- **GIVEN** the lab runs a derived high-loss variant of `core-01`
- **WHEN** the data plane uses `kcp`
- **THEN** the traversal attempt succeeds
- **AND** diagnostics contain `stage=transport kind=ok msg="kcp payload exchanged"`

#### Scenario: Validate QUIC payload exchange under high loss
- **GIVEN** the lab runs a derived high-loss variant of `core-01`
- **WHEN** the data plane uses `quic`
- **THEN** the traversal attempt succeeds
- **AND** diagnostics contain `stage=transport kind=ok msg="quic payload exchanged"`
