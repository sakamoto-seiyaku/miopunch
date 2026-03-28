## ADDED Requirements

### Requirement: Derived P2 Path Regression Coverage
The system SHALL provide derived lab regressions for representative `P2` path selections without changing the `P0` base NAT matrix.

#### Scenario: Keep the existing representative P2 paths under strict validation
- **GIVEN** the lab runs the representative `P2` cases for `IPv6 direct`, `IPv4 portmap direct`, and `IPv4 punching fallback`
- **WHEN** the regression output is evaluated
- **THEN** each case is checked against its expected ordered evidence chain
- **AND** each successful case also requires transport payload-exchange evidence

#### Scenario: Fall back from IPv6 direct to IPv4 direct when IPv6 is present but unreachable
- **GIVEN** both peers gather `IPv6` candidates and `IPv4 portmap` candidates
- **AND** the `IPv6` path is not reachable
- **WHEN** the attempt phase runs
- **THEN** diagnostics contain `attempt.v6.start` followed by `attempt.v6.fail`
- **AND** diagnostics then contain `attempt.v4.start` followed by `attempt.v4.ok`
- **AND** the run is accepted only if the transport stage also exchanges application payload
