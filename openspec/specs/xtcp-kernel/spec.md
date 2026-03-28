# xtcp-kernel Specification

## Purpose
`xtcp-kernel` 定义并约束 `P1/P3` 阶段的“打洞内核”：以中心协调（control plane）协助两端协商并产出可用的 P2P UDP path，同时提供最小 UDP self-check 与阶段化、可机读的可观测事件流。打洞成功后的数据面会话建立与 `payload exchanged` 验收由 `miopunch-dataplane` 承诺与约束。
## Requirements
### Requirement: Coordinator-Assisted UDP Traversal Kernel
The system SHALL provide a coordinator-assisted UDP traversal workflow that can negotiate a P2P session between two peers behind NAT.

#### Scenario: Establish a session in a representative easy NAT case
- **GIVEN** a NAT lab environment is available with a representative `NAT1 x NAT1` case (e.g., `core-01`)
- **WHEN** two peers run the `xtcp-kernel` CLI against a coordinator to establish a session
- **THEN** the peers establish a P2P UDP session
- **AND** the peers perform a minimal UDP self-check to confirm the path is usable

### Requirement: Stage-Level Observability
The system SHALL expose a stage-based, machine-readable timeline for each traversal attempt.

#### Scenario: Explain a failure by stage
- **GIVEN** a traversal attempt fails in a NAT lab case
- **WHEN** the developer inspects the run output and artifacts
- **THEN** the failure is attributed to a specific stage
- **AND** the system records relevant conditions (candidates, retries, timeouts, and decisions) needed for diagnosis

### Requirement: No Fallback Relay in P1
The system SHALL NOT perform fallback relay in `P1`.
If a direct P2P session cannot be established, the attempt SHALL fail with stage-level diagnostics.

#### Scenario: Failure is explicit without relay
- **GIVEN** a traversal attempt cannot establish a direct P2P session
- **WHEN** the attempt ends
- **THEN** the result is reported as a direct-connect failure
- **AND** the output includes stage-level diagnostics for troubleshooting

### Requirement: NAT Lab Regression Entry Point
The system SHALL provide a repeatable integration test entry point that runs against the `P0` NAT lab testbed.

#### Scenario: Run representative lab cases
- **GIVEN** the `P0` NAT lab testbed is available
- **WHEN** the integration regression suite is executed
- **THEN** representative success and failure paths are exercised
- **AND** artifacts required for diagnosis are collected on failure

### Requirement: Control Plane Transport Options
The system SHALL support selecting the `control plane` transport protocol between `KCP` and `QUIC`, with `TCP` as a baseline default.
This requirement applies to coordinator signaling and does not include `fallback relay`.

#### Scenario: Connect to coordinator using QUIC
- **GIVEN** the developer configures `quic` as the `control plane` transport
- **WHEN** a peer connects to the coordinator
- **THEN** the control plane connection is established using QUIC
- **AND** the selected transport is visible in machine-readable diagnostics

#### Scenario: Connect to coordinator using KCP
- **GIVEN** the developer configures `kcp` as the `control plane` transport
- **WHEN** a peer connects to the coordinator
- **THEN** the control plane connection is established using KCP
- **AND** the selected transport is visible in machine-readable diagnostics

