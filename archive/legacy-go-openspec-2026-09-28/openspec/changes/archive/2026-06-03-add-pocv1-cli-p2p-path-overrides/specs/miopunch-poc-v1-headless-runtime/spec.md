## ADDED Requirements

### Requirement: Current v1 peer actions carry per-command P2P path policy
The current v1 runtime SHALL accept per-command P2P path policy on peer actions that establish or reuse a peer session.

The policy SHALL include:

- `p2p_network`: `auto`, `udp_only`, or `tcp_only`
- `p2p_ip_family`: `auto`, `v4`, or `v6`

The current v1 CLI SHALL expose this policy for `ping`, `sh ls`, and `sh` through short flags and long options.

The runtime SHALL treat an omitted policy as `auto` and preserve existing default behavior.

#### Scenario: Ping carries IPv4-only policy into runtime
- **WHEN** a user runs current v1 `miopunch ping <peer> -4`
- **THEN** the action arguments carry `p2p_ip_family=v4`
- **AND** peer session establishment receives that policy instead of using the default family behavior

#### Scenario: Shell list carries UDP-only policy into runtime
- **WHEN** a user runs current v1 `miopunch sh ls <peer> -u`
- **THEN** the action arguments carry `p2p_network=udp_only`
- **AND** peer session establishment receives that policy instead of ignoring the CLI option

#### Scenario: Omitted policy remains automatic
- **WHEN** a user runs current v1 `miopunch ping <peer>` without `-u`, `-t`, `-4`, `-6`, `--p2p-network`, or `--p2p-ip-family`
- **THEN** the runtime uses automatic P2P path behavior
- **AND** existing default command behavior is preserved

### Requirement: Explicit P2P path policy constrains peer session reuse
The current v1 runtime SHALL NOT reuse an existing peer session for a command with explicit P2P path policy unless the existing session satisfies that policy.

If an existing session does not satisfy the explicit policy, the runtime SHALL establish a fresh peer session under the requested policy.

If no explicit P2P path policy is supplied, the runtime MAY reuse a healthy existing peer session as before.

#### Scenario: IPv4-only command does not reuse IPv6 session
- **GIVEN** a healthy peer session already exists with selected path `direct_ipv6`
- **WHEN** a user runs current v1 `miopunch ping <peer> -4`
- **THEN** the runtime does not reuse the existing IPv6 peer session
- **AND** it establishes a fresh session under IPv4-only P2P policy

#### Scenario: Default command may reuse healthy session
- **GIVEN** a healthy peer session already exists for a peer
- **WHEN** a user runs current v1 `miopunch ping <peer>` without explicit P2P path policy
- **THEN** the runtime may reuse the existing healthy peer session

### Requirement: Current v1 reports unsupported explicit TCP-only path policy
The current v1 runtime SHALL reject an explicit `tcp_only` P2P path policy with an actionable unsupported-path failure.

The runtime SHALL NOT silently fall back to UDP when the command explicitly requested `tcp_only`.

#### Scenario: TCP-only ping fails explicitly
- **WHEN** a user runs current v1 `miopunch ping <peer> -t`
- **THEN** the command fails before UDP path establishment
- **AND** the failure explains that current POC v1 does not support TCP-only P2P path establishment

#### Scenario: TCP-only shell fails explicitly
- **WHEN** a user runs current v1 `miopunch sh <peer> -t`
- **THEN** the command fails before shell attach
- **AND** the failure explains that current POC v1 does not support TCP-only P2P path establishment
