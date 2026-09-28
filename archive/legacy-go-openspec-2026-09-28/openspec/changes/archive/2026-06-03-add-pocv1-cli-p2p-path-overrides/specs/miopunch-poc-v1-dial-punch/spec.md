## ADDED Requirements

### Requirement: Current v1 UDP path establishment honors requested P2P IP family
Current v1 UDP direct-first dial/punch SHALL honor the requested per-command `p2p_ip_family` policy when gathering candidates and attempting peer paths.

When `p2p_ip_family=v4`, current v1 UDP path establishment SHALL gather and attempt only IPv4 P2P candidates.

When `p2p_ip_family=v6`, current v1 UDP path establishment SHALL gather and attempt only IPv6 P2P candidates.

When `p2p_ip_family=auto` or omitted, current v1 UDP path establishment SHALL preserve existing automatic candidate behavior.

#### Scenario: IPv4-only policy excludes IPv6 direct path
- **WHEN** current v1 path establishment runs with `p2p_ip_family=v4`
- **THEN** it does not select `direct_ipv6`
- **AND** it gathers and attempts only IPv4 P2P candidates

#### Scenario: IPv6-only policy excludes IPv4 path establishment
- **WHEN** current v1 path establishment runs with `p2p_ip_family=v6`
- **THEN** it does not select `direct_ipv4`
- **AND** it does not select `punching_ipv4`
- **AND** it gathers and attempts only IPv6 P2P candidates

#### Scenario: Automatic policy preserves default behavior
- **WHEN** current v1 path establishment runs without an explicit P2P IP-family policy
- **THEN** it uses the existing automatic candidate gathering and attempt behavior

### Requirement: Current v1 UDP path establishment honors requested P2P network policy
Current v1 UDP direct-first dial/punch SHALL honor the requested per-command `p2p_network` policy.

When `p2p_network=udp_only` or `auto`, current v1 path establishment MAY use the existing UDP direct-first and UDP punching behavior.

When `p2p_network=tcp_only`, current v1 path establishment SHALL fail with an explicit unsupported-path result and SHALL NOT silently run UDP fallback.

#### Scenario: UDP-only policy uses UDP path establishment
- **WHEN** current v1 path establishment runs with `p2p_network=udp_only`
- **THEN** it uses UDP direct-first and UDP punching fallback according to the current v1 dial/punch rules

#### Scenario: TCP-only policy is rejected
- **WHEN** current v1 path establishment runs with `p2p_network=tcp_only`
- **THEN** it fails with an unsupported-path result
- **AND** it does not run UDP direct reachability or UDP punching as a silent fallback
