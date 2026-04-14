## ADDED Requirements

### Requirement: Provide case0-4 verification runbook
The project SHALL provide a documented manual verification matrix for real-network testing (case0-4). The runbook MUST define, per case: endpoints, role assignment (`peer client` vs `peer visitor`), required dependencies (STUN + MQTT), and success criteria.

#### Scenario: Operator can identify what to run
- **WHEN** the operator opens the runbook
- **THEN** it lists case0, case1, case2, case3, and case4 with clear endpoint/role mapping

### Requirement: Provide YAML config templates for running cases
The project SHALL provide YAML configuration templates suitable for `miopunch peer client --config <yaml>` and `miopunch peer visitor --config <yaml>`. Templates MUST include placeholders for MQTT broker settings and STUN servers. Secrets (MQTT credentials, `secret`) MUST NOT be committed as real values.

#### Scenario: Operator can bootstrap configs without long CLI
- **WHEN** the operator copies a template and fills required values
- **THEN** `miopunch peer client|visitor --config <yaml>` starts without requiring long flag lists

### Requirement: Successful runs emit payload-exchanged evidence
For any case expected to succeed, both endpoints MUST emit an `ok` event named `transport.payload_exchanged` to serve as the primary acceptance evidence.

#### Scenario: Success is verifiable from logs
- **WHEN** a case completes successfully
- **THEN** both sides' logs contain `transport.payload_exchanged`

### Requirement: Android arm64 build and ADB deploy are documented
The runbook SHALL include steps to cross-compile an Android arm64 `miopunch` binary and deploy/execute it via ADB without requiring Go to be installed on the Android device.

#### Scenario: Operator can run miopunch on Pixel6a
- **WHEN** the operator follows the Android build/deploy instructions
- **THEN** the Pixel6a can execute the `miopunch` binary and print command help

### Requirement: Case0 LAN smoke test is included
The runbook MUST include case0 as a LAN smoke test between Host and Pixel6a (same LAN) that validates end-to-end payload exchange.

#### Scenario: LAN baseline succeeds
- **WHEN** the operator runs case0 on Host and Pixel6a within the configured overall timeout
- **THEN** both endpoints emit `transport.payload_exchanged`
