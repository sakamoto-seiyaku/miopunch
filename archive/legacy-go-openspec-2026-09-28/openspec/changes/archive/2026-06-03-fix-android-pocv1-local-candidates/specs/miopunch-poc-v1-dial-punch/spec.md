## ADDED Requirements

### Requirement: Android local candidates avoid standard interface enumeration
Current v1 UDP path establishment SHALL gather Android app local direct
candidates without calling Go standard `net.Interfaces()` or
`net.InterfaceAddrs()` on Android builds.

Android local candidate gathering SHALL attempt an Android-safe address provider
that does not require MAC addresses, `RTM_GETLINK`, or a bound
`NETLINK_ROUTE` socket.

If Android local candidate gathering returns no usable direct addresses, the
system SHALL report that condition in trace diagnostics and SHALL NOT silently
publish `127.0.0.1` as a peer-reachable direct candidate.

#### Scenario: Android app sandbox blocks standard enumeration
- **WHEN** an Android build gathers current v1 local candidates
- **THEN** it does not invoke Go standard interface enumeration
- **AND** it records Android provider diagnostics for the candidate source used

#### Scenario: Android provider has no usable address
- **WHEN** Android local candidate gathering returns no peer-reachable address
- **THEN** the candidate set excludes loopback fallback addresses
- **AND** trace diagnostics identify that no usable Android local candidate was found

### Requirement: Route-source derivation supplements peer direct candidates
Current v1 UDP path establishment SHALL be able to derive local host candidates
from known peer direct targets by using the kernel-selected UDP source address
for those targets.

The derived candidate SHALL use the runtime UDP socket port for the matching IP
family and SHALL be subject to the same IP-family policy filtering as other
current v1 direct candidates.

Derived candidates SHALL supplement, not replace, enumerated local candidates.

#### Scenario: Peer target reveals Android local source address
- **WHEN** Android receives a peer direct target that is route-reachable
- **THEN** the runtime derives the local source IP selected for that target
- **AND** it adds that local source IP with the runtime UDP port as a host candidate

#### Scenario: Route-source derivation fails for a target
- **WHEN** a peer direct target cannot be dialed for route-source derivation
- **THEN** the runtime records trace diagnostics for that target
- **AND** it continues with the remaining candidate sources
