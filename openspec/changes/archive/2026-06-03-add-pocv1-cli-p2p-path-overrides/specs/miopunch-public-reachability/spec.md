## MODIFIED Requirements

### Requirement: P2P IP Family Override Flags
The system SHALL support short flags `-4` and `-6` on peer commands to constrain the `P2P/打洞` address family.
These flags SHALL NOT constrain signaling connectivity such as MQTT, enrollment, invitation, approval, roster lookup, or control-plane message delivery.

The system SHALL also support a long-form `--p2p-ip-family` option with at least `auto`, `v4`, and `v6` values on current POC v1 peer commands that establish or reuse P2P peer sessions.

The system SHALL reject conflicting explicit IP-family flags in the same command.

#### Scenario: Force IPv4-only P2P without constraining signaling
- **WHEN** a peer command is run with `-4`
- **THEN** it gathers and attempts only IPv4 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Force IPv6-only P2P without constraining signaling
- **WHEN** a peer command is run with `-6`
- **THEN** it gathers and attempts only IPv6 P2P candidates
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Long-form IP family policy matches short flags
- **WHEN** a peer command is run with `--p2p-ip-family v4`
- **THEN** it behaves as though the command was run with `-4`
- **AND** the signaling layer is still allowed to connect using any available IP family

#### Scenario: Conflicting family flags fail early
- **WHEN** a peer command is run with both `-4` and `-6`
- **THEN** the command fails with an argument error
- **AND** no P2P path establishment is attempted
