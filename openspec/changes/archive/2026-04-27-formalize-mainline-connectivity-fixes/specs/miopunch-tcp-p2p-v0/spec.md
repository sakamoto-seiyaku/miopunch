## ADDED Requirements

### Requirement: TCP private addresses use assisted candidate semantics
The system SHALL distinguish TCP direct candidates from TCP assisted/private punching inputs.

`tcp_direct_addrs` SHALL contain only TCP addresses that are valid direct path candidates. Private or local TCP listen addresses that may help punching SHALL be exchanged as `tcp_assisted_addrs` and SHALL NOT be attempted by `direct_tcp4`.

#### Scenario: Private TCP listen address is not direct
- **WHEN** gather observes a private IPv4 TCP listen address behind NAT
- **THEN** the address is not emitted as a TCP direct candidate
- **AND** it can be emitted as a TCP assisted punching input when assisted exchange is enabled

### Requirement: TCP assisted punching is diagnostic and bounded
When TCP STUN evidence is insufficient but explicit assisted TCP targets exist, the system SHALL allow a bounded best-effort TCP punching fallback.

The fallback SHALL NOT claim to have completed NAT feature analysis, SHALL NOT apply random/range spraying to assisted targets, and SHALL report successful payload exchange as `punching_tcp4`.

#### Scenario: Assisted-only TCP fallback succeeds as punching
- **GIVEN** a peer has no sufficient TCP STUN evidence
- **AND** the peer has one or more TCP assisted targets
- **WHEN** TCP direct paths fail and assisted TCP punching succeeds
- **THEN** the selected path is reported as `punching_tcp4`
- **AND** diagnostics identify the winning target as assisted
