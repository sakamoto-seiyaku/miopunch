## MODIFIED Requirements

### Requirement: Current v1 path establishment uses UDP direct-first with punching fallback
The system SHALL establish current POC v1 peer-to-peer paths using UDP carrier semantics only.

For UDP direct candidates, the system SHALL attempt direct reachability before UDP punching.

If UDP direct reachability succeeds, the system SHALL select that UDP path and SHALL NOT run UDP punching for that candidate set.

If UDP direct reachability fails, the system SHALL fall back to UDP punching using the decision-derived `NatHoleResp` behavior within the bounded attempt budget.

The system SHALL preserve archived UDP semantics for ordinary STUN mapped address sampling, assisted/private candidates, UDP mode0..4 behavior, `CandidatePorts`, `SendRandomPorts`, and `ListenRandomPorts`.

The system SHALL support UDP6 direct paths when local and peer UDP6 candidates are available, unless an explicit POC v1 IPv4-only decision is recorded in this change.

For UDP6 direct paths, the system SHALL treat exchanged IPv6 direct addresses as reachability candidates and SHALL select the path using the observed remote endpoint produced by SID probing.

The system SHALL preserve both global IPv6 and ULA IPv6 direct candidates within the bounded direct-candidate limit.

The system SHALL NOT branch to TCP, relay, QUIC, or other carrier types in this capability.

#### Scenario: Same-LAN host candidates use UDP direct first
- **WHEN** a current v1 peer tries to establish a path to a UDP host candidate that is directly reachable
- **THEN** it selects the UDP direct path
- **AND** it records the selected path as `direct_ipv4` or `direct_ipv6`
- **AND** it does not require UDP punching to converge for that candidate

#### Scenario: Direct timeout falls back to UDP punching
- **WHEN** a current v1 peer tries UDP direct reachability
- **AND** the direct attempt times out
- **THEN** it attempts UDP punching using the decision-derived `NatHoleResp`
- **AND** it preserves failure evidence for the direct attempt

#### Scenario: Path establishment remains UDP-only
- **WHEN** a current v1 peer tries to establish a path
- **THEN** it uses only UDP direct reachability and UDP punching
- **AND** it does not use TCP, relay, QUIC, or another carrier inside this capability

#### Scenario: Mode2 or mode4 can select a temporary random-listen winner
- **WHEN** the UDP decision behavior requests `ListenRandomPorts`
- **AND** a temporary random-listen UDP socket receives the winning SID exchange
- **THEN** current v1 selects that temporary UDP socket as the path owner
- **AND** it does not relabel the temporary winner as the Runtime-owned UDP socket

#### Scenario: Direct IPv6 nominates the observed endpoint
- **WHEN** a current v1 peer exchanges multiple IPv6 direct candidates with another peer
- **AND** SID probing succeeds from an IPv6 endpoint that is not the first candidate in the peer list
- **THEN** current v1 records `direct_ipv6` as the selected path
- **AND** `PathResult` includes the observed remote UDP endpoint and the bounded allowed IPv6 direct endpoint set for secure-session handoff

### Requirement: Current v1 candidate exchange uses only dial_offer and dial_answer
The system SHALL exchange punch parameters using only `dial_offer` and `dial_answer` sent over the recipient's roster-backed inbox topic via `miopunch-poc-v1-controlplane-wire`.

Recipient inbox addressing and recipient X25519 selection SHALL be resolved from the persisted current v1 roster plus current v1 topic derivation state, not from presence payload contents.

The body field set SHALL include:

- `dial_id`
- `punch_token`
- local candidates used for evidence
- UDP gather snapshot containing direct, mapped, and assisted UDP addresses
- `member_credential`

The `dial_answer` body SHALL also include the UDP decision material needed by both peers to run their assigned `NatHoleResp` behavior.

The receiving peer SHALL treat body `member_credential` as an asserted identity that still requires validation.
It SHALL accept that credential only when:

- inner `sender_peer_id` matches the credential-derived `peer_id`
- inner `sender_ed25519` matches `member_credential.subject_ed25519_pub`
- the credential matches the trusted roster entry for that `peer_id`
- the credential verifies under the network authority Ed25519 public key

#### Scenario: Candidate exchange stays within the fixed two-message flow
- **WHEN** two enrolled peers coordinate a current v1 punch attempt
- **THEN** they exchange only `dial_offer` and `dial_answer`
- **AND** those messages carry UDP snapshot and decision material inside the authenticated bodies

#### Scenario: Candidate exchange rejects invalid asserted identity
- **WHEN** a receiving peer opens a `dial_offer` or `dial_answer`
- **AND** the asserted `member_credential` disagrees with inner sender identity, trusted roster bytes, or authority verification
- **THEN** current v1 dial/punch rejects that message
- **AND** the remote identity does not enter `PathResult`

#### Scenario: Dial answer stays bound to the current exchange
- **WHEN** a dialer receives a `dial_answer`
- **AND** its `dial_id`, `punch_token`, `in_reply_to`, or responder `peer_id` does not match the in-flight offer
- **THEN** current v1 dial/punch rejects that answer
- **AND** it does not bind that answer to the current punch attempt

### Requirement: PathResult is the only output of this capability
The system SHALL output `PathResult` from current v1 dial/punch.

`PathResult` SHALL contain only:

- the selected UDP path and selected remote UDP endpoint
- for direct IPv6, the bounded remote UDP endpoints acceptable during secure-session handoff
- explicit selected UDP socket ownership / handoff information
- the trusted remote identity handoff (`peer_id` and `MemberCredential`)
- punch evidence, including the selected UDP path kind

`PathResult` SHALL distinguish at least:

- Runtime-owned selected UDP path, where the next layer must use the Runtime owner/demux transport view
- temporary selected UDP path, where the next layer owns the selected UDP socket after handoff

`PathResult` SHALL be a closed handoff for `miopunch-poc-v1-secure-session`; that next layer SHALL NOT need to reopen roster lookup or dial target selection to recover remote identity.
The system SHALL NOT embed KCP/TLS/yamux or session-recipe selection into `PathResult`.

#### Scenario: Session upgrade remains outside dial/punch
- **WHEN** current v1 dial/punch succeeds
- **THEN** the result is a `PathResult`
- **AND** the session layer still chooses how to upgrade that path without re-reading roster authority
- **AND** the remote identity in that result has already passed sender, roster, and authority validation

#### Scenario: PathResult identifies the selected UDP path kind
- **WHEN** current v1 dial/punch succeeds through UDP direct reachability
- **THEN** the resulting punch evidence identifies `direct_ipv4` or `direct_ipv6` as the selected path
- **AND** when it succeeds through UDP punching, the resulting punch evidence identifies `punching_ipv4` as the selected path

#### Scenario: Temporary winner is not treated as a borrowed Runtime socket
- **WHEN** current v1 dial/punch selects a temporary random-listen UDP socket
- **THEN** `PathResult` marks that socket as selected-path owned
- **AND** failed secure-session handoff can close that temporary socket without closing Runtime's UDP owner

## ADDED Requirements

### Requirement: POC v1 UDP gather preserves archived assisted candidate semantics
The system SHALL derive POC v1 assisted/private UDP candidates from archived UDP gather semantics.

The system SHALL NOT add POC v1-specific interface-name filtering for assisted UDP candidates unless a separate explicit decision records that exception.

#### Scenario: Assisted candidates are not filtered by virtual interface name
- **WHEN** current v1 UDP gather observes a non-loopback, non-link-local IPv4 address on a virtual or bridge-style interface
- **THEN** the assisted candidate is eligible under the archived gather semantics
- **AND** POC v1 runtime does not drop it solely because the interface name matches Docker, bridge, veth, CNI, virbr, or Hyper-V default switch patterns

### Requirement: POC v1 records local analyzer metadata for UDP success
The current v1 dial/punch result SHALL include or allow recomputation of local analyzer metadata needed to report UDP punching success under the local daemon's remote-peer/protocol scope.

The initiator SHALL NOT report UDP success using analyzer metadata that is scoped only to the responder's local view.

#### Scenario: Initiator success uses local analyzer scope
- **WHEN** the initiator succeeds with a UDP punching mode/index
- **THEN** it reports success under the initiator daemon's local remote-peer/protocol scope
- **AND** it does not write success into the responder daemon's analyzer key

#### Scenario: Responder success uses local analyzer scope
- **WHEN** the responder succeeds with a UDP punching mode/index
- **THEN** it reports success under the responder daemon's local remote-peer/protocol scope
- **AND** later responder attempts can use that local success memory
