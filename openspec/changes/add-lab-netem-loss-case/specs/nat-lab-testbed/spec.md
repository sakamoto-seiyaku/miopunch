## ADDED Requirements

### Requirement: Derived Regression Variants
The system SHALL allow derived regression cases that reuse an existing NAT baseline while varying helper availability, IPv6 reachability, or link conditions, without changing the `P0` core NAT case matrix.

#### Scenario: Add a P2 derived case without mutating the core matrix
- **GIVEN** the `P0` core NAT matrix is already fixed as the baseline regression set
- **WHEN** a new `P2` connectivity path needs extra coverage
- **THEN** the lab adds that coverage as a derived case or derived run variant
- **AND** the existing `core-01..core-10` baseline cases remain unchanged

#### Scenario: Add a loss-conditioned transport variant without expanding NAT combinations
- **GIVEN** the transport regression needs a representative high-loss case
- **WHEN** the lab adds the new coverage
- **THEN** it reuses an existing baseline NAT case such as `core-01`
- **AND** it expresses packet loss as a derived variant rather than a new NAT matrix entry

### Requirement: Strict Case Output Validation
The system SHALL allow a lab regression case to declare ordered machine-readable output expectations and required transport evidence that must be satisfied before the run is marked passing.

#### Scenario: Validate an ordered fallback chain
- **GIVEN** a regression case expects a specific connectivity fallback chain
- **WHEN** the run output is evaluated
- **THEN** the validator checks the required events in order
- **AND** the case does not pass if the expected order is missing or shuffled

#### Scenario: Require payload evidence for a successful data path
- **GIVEN** a regression case is expected to exchange application payload after connectivity succeeds
- **WHEN** the run output is evaluated
- **THEN** the validator requires a `stage=transport kind=ok` payload-exchange event
- **AND** the case does not pass on process exit code alone
