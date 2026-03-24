# xtcp-kernel Specification

## Purpose
`xtcp-kernel` 定义并约束 `P1` 阶段的 XTCP 打洞内核：以“中心协调（control plane）+ 两端 P2P 数据面”为基础形态，实现 `IPv4 UDP` 打洞（不包含 relay/fallback），并提供阶段化、可机读的可观测事件流；同时在 `P0 nat-lab-testbed` 中提供可重复的集成回归入口，以证明核心算法行为基线稳定且可复盘。
## Requirements
### Requirement: Coordinator-Assisted UDP Traversal Kernel
The system SHALL provide a coordinator-assisted UDP traversal workflow that can negotiate a P2P session between two peers behind NAT.

#### Scenario: Establish a session in a representative easy NAT case
- **GIVEN** a NAT lab environment is available with a representative `NAT1 x NAT1` case (e.g., `core-01`)
- **WHEN** two peers run the `xtcp-kernel` CLI against a coordinator to establish a session
- **THEN** the peers establish a P2P UDP session
- **AND** the peers can exchange application payload over that session

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

### Requirement: P2P Data Plane Transport Selection
The system SHALL support selecting the direct P2P data plane transport protocol between `KCP` and `QUIC` after UDP hole punching succeeds.
This requirement applies to the direct P2P session and does not require `fallback relay` to use the same transport.

#### Scenario: Establish P2P session using QUIC
- **GIVEN** the system negotiates `quic` as the P2P data plane transport
- **WHEN** the peers establish the P2P session
- **THEN** the peers can exchange application payload over QUIC streams
- **AND** the selected transport is visible in machine-readable diagnostics

#### Scenario: Establish P2P session using KCP
- **GIVEN** the system negotiates `kcp` as the P2P data plane transport
- **WHEN** the peers establish the P2P session
- **THEN** the peers can exchange application payload over KCP-based streams
- **AND** the selected transport is visible in machine-readable diagnostics
