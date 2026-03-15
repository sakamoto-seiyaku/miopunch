## ADDED Requirements

### Requirement: Isolated Single-VM Lab Host
The system SHALL provide a NAT lab testbed that runs inside a single `QEMU VM` and does not require persistent modifications to the default `Windows` or `WSL2` network.

#### Scenario: Start isolated lab host
- **GIVEN** the developer is working from `Windows 11 + WSL2`
- **WHEN** the NAT lab testbed is prepared
- **THEN** the lab host runs inside one `QEMU VM`
- **AND** the default `Windows` and `WSL2` network remain outside the test topology

### Requirement: Managed Baseline and Snapshots
The system SHALL support recoverable baseline states for the NAT lab environment.

#### Scenario: Restore clean baseline
- **GIVEN** the lab host has been modified by previous experiments
- **WHEN** the developer restores a baseline snapshot
- **THEN** the environment returns to a clean, known-good state
- **AND** the snapshot represents an environment baseline rather than a single test result

#### Scenario: Distinguish baseline levels
- **GIVEN** the NAT lab is being prepared for repeated use
- **WHEN** baseline snapshots are created
- **THEN** there is a `base-ready` state with VM access and tools installed
- **AND** there is a `lab-ready` state with case definitions and switching logic installed but no active case or test binary present

### Requirement: Case-Based NAT Topology Execution
The system SHALL support multiple NAT lab case definitions within one VM while permitting only one active case at a time.

#### Scenario: Switch active case
- **GIVEN** multiple case definitions exist in the VM
- **WHEN** the developer activates a specific case
- **THEN** only that case's topology is active
- **AND** other case definitions remain inactive until selected

#### Scenario: Refine a case after observed asymmetry
- **GIVEN** a representative case is defined without A/B direction splitting
- **WHEN** testing shows a clear behavioral difference based on initiator order, role assignment, or direction
- **THEN** that representative case may be split into smaller test cases
- **AND** the split is driven by observed behavior rather than by default enumeration

### Requirement: Triple-Labeled NAT Case Classification
The system SHALL classify each NAT case using RFC 4787 behavior as the primary model, while also recording `NAT1-4` compatibility labels and `frp` engineering labels.

#### Scenario: Record a NAT case
- **GIVEN** a NAT case is defined in the lab
- **WHEN** the case metadata is recorded
- **THEN** it includes RFC 4787 mapping and filtering behavior
- **AND** it includes a `NAT1-4` compatibility label when applicable
- **AND** it includes the corresponding `frp` engineering view such as `EasyNAT` or `HardNAT`
- **AND** it includes a stable case identifier
- **AND** it records A-side and B-side labels separately
- **AND** it records initiator order and role assignment decisions relevant to the run

#### Scenario: Handle RFC 4787 combinations beyond NAT1-4
- **GIVEN** a case matches RFC 4787 behavior that does not naturally fit `NAT1-4`
- **WHEN** the case is documented
- **THEN** RFC 4787 behavior remains the source-of-truth label
- **AND** the case is not forced into an incorrect `NAT1-4` bucket

### Requirement: Verified NAT Profiles
The system SHALL verify that the configured NAT profiles behave like their claimed labels before they are used as trusted regression cases.

#### Scenario: Validate claimed NAT behavior
- **GIVEN** a case is labeled as a specific RFC 4787 and `NAT1-4` profile
- **WHEN** the case validation process runs
- **THEN** the resulting mapping and filtering behavior is checked against the claimed profile
- **AND** the case is only considered trusted when the observed behavior matches the intended label set

### Requirement: Observable Failure and State Artifacts
The system SHALL produce enough observability data to explain lab behavior and diagnose failures.

#### Scenario: Diagnose a failed traversal attempt
- **GIVEN** a traversal attempt fails inside a lab case
- **WHEN** the failure is inspected
- **THEN** the developer can identify the failed stage
- **AND** the lab exposes relevant logs, packet captures, rule state, connection state, and link state needed for diagnosis
- **AND** the lab produces a minimal artifact set including `pcap`, NAT ruleset (`nft`/`iptables`), `conntrack` snapshot, and `tc qdisc` stats
