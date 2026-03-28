## MODIFIED Requirements

### Requirement: Coordinator-Assisted UDP Traversal Kernel
The system SHALL provide a coordinator-assisted UDP traversal workflow that can negotiate a P2P session between two peers behind NAT.
This capability SHALL only guarantee a usable P2P UDP path and a minimal self-check over UDP datagrams.
Post-connectivity data plane transport selection and application payload exchange are out of scope here (see `miopunch-dataplane`).

#### Scenario: Establish a session in a representative easy NAT case
- **GIVEN** a NAT lab environment is available with a representative `NAT1 x NAT1` case (e.g., `core-01`)
- **WHEN** two peers run the `xtcp-kernel` CLI against a coordinator to establish a session
- **THEN** the peers establish a P2P UDP session
- **AND** the peers perform a minimal UDP self-check to confirm the path is usable

## REMOVED Requirements

### Requirement: P2P Data Plane Transport Selection
**Reason**: `P3` introduces `miopunch-dataplane` to own post-connectivity data plane responsibilities; keeping data plane selection in `xtcp-kernel` blurs the boundary and makes failures harder to attribute.
**Migration**: Use `miopunch-dataplane` for selecting `kcp` vs `quic` (and `quic-cc=bbr|brutal`) and for validating `payload exchanged` evidence and transport statistics.

