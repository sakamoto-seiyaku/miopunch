## ADDED Requirements

### Requirement: TCP gather separates direct and assisted addresses
When TCP is permitted, gather SHALL emit true TCP direct candidates separately from TCP assisted/private candidates.

Private IPv4, loopback, link-local, unspecified, and other non-direct TCP listen addresses SHALL NOT be emitted as `tcp_direct_addrs`. Eligible private/local TCP listen addresses MAY be emitted as `tcp_assisted_addrs` when assisted exchange is enabled.

#### Scenario: Private TCP address becomes assisted
- **WHEN** a peer behind NAT listens on a private IPv4 TCP address
- **THEN** gather does not include that address in `tcp_direct_addrs`
- **AND** gather can include it in `tcp_assisted_addrs`

### Requirement: TCP decision preserves direct and assisted buckets
The punching decision boundary SHALL preserve separate TCP direct, assisted exact, candidate exact, and candidate-expanded target buckets.

`direct_tcp4` SHALL consume only peer TCP direct addresses. `punching_tcp4` SHALL consume assisted exact targets and TCP candidate targets according to their bucket semantics.

#### Scenario: Direct attempt skips assisted targets
- **GIVEN** a response contains both peer TCP direct addresses and TCP assisted addresses
- **WHEN** the attempt runs `direct_tcp4`
- **THEN** it attempts only peer TCP direct addresses
- **AND** assisted addresses remain available only to TCP punching

### Requirement: Assisted-only TCP punching is bounded and explicit
When TCP STUN evidence is insufficient but TCP assisted targets exist, the system SHALL support an explicit bounded assisted-only TCP punching fallback.

The fallback SHALL use minimal mode0 behavior, SHALL NOT apply range/random port expansion to assisted targets, and SHALL emit diagnostics explaining that NAT analysis was unavailable.

#### Scenario: Assisted-only fallback reports source
- **WHEN** assisted-only TCP punching succeeds
- **THEN** the selected path is `punching_tcp4`
- **AND** diagnostics report that the winning target source was assisted

### Requirement: MNT-01 TCP direct cases require true direct candidates
MNT-01 cases SHALL NOT assert `direct_tcp4` success unless the fixture produces true TCP direct candidates.

Cases that validate fallback through private assisted targets SHALL be named and asserted as TCP punching fallback cases.

#### Scenario: Existing double-NAT TCP fallback case is not direct
- **WHEN** a double-NAT fixture provides only private TCP listen addresses
- **THEN** the case does not require `direct_tcp4`
- **AND** successful fallback is asserted as `punching_tcp4`
