## ADDED Requirements

### Requirement: Invite broker emission uses hostname-preserving broker selection
The invite task SHALL use a hostname-preserving broker selection path when
emitting `invite_brokers` in invite codes.

The invite task SHALL NOT use a helper that resolves reachable hostname broker
endpoints to A-record IP addresses for invite-code output.

#### Scenario: Reachable hostname broker is not IP-canonicalized for invite code output
- **WHEN** an invite broker candidate is provided as a reachable `host:port`
- **THEN** the emitted invite code contains the selected normalized `host:port`
- **AND** the emitted invite code does not replace that hostname with a resolved
  IP address

#### Scenario: IP-canonicalizing helper is not used by invite emission
- **WHEN** the invite task selects reachable broker endpoints for invite-code
  output
- **THEN** the implementation path preserves the selected endpoint string after
  validation and reachability probing
- **AND** helpers that resolve hostnames to IP addresses are not used for the
  emitted `invite_brokers` value
