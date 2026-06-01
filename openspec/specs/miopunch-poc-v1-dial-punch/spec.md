# miopunch-poc-v1-dial-punch Specification

## Purpose
定义当前 POC v1 的 UDP direct-first dial/punch：`dial_offer`、`dial_answer`、bounded attempt runtime 与 `PathResult`。

## Requirements

### Requirement: Current v1 path establishment uses UDP direct-first with punching fallback
The system SHALL establish current POC v1 peer-to-peer paths using UDP carrier semantics only.

For IPv4 host-to-host candidate pairs, the system SHALL attempt UDP direct reachability before UDP punching.

If UDP direct reachability succeeds, the system SHALL select that UDP path and SHALL NOT run UDP punching for that pair.

If UDP direct reachability fails, the system SHALL fall back to UDP punching within the existing bounded attempt budget.

The system SHALL NOT branch to TCP, relay, QUIC, or other carrier types in this capability.

#### Scenario: Same-LAN host candidates use UDP direct first
- **WHEN** a current v1 peer tries to establish a path to an IPv4 host candidate that is directly reachable over UDP
- **THEN** it selects the UDP direct path
- **AND** it records the selected path as `direct_ipv4`
- **AND** it does not require UDP punching to converge for that pair

#### Scenario: Direct timeout falls back to UDP punching
- **WHEN** a current v1 peer tries UDP direct reachability for a host candidate pair
- **AND** the direct attempt times out
- **THEN** it attempts UDP punching for the same candidate pair
- **AND** it preserves failure evidence for the direct attempt

#### Scenario: Path establishment remains UDP-only
- **WHEN** a current v1 peer tries to establish a direct path
- **THEN** it uses only UDP direct reachability and UDP punching
- **AND** it does not use TCP, relay, QUIC, or another carrier inside this change

### Requirement: Current v1 candidate exchange uses only dial_offer and dial_answer
The system SHALL exchange punch parameters using only `dial_offer` and `dial_answer` sent over the recipient's roster-backed inbox topic via `miopunch-poc-v1-controlplane-wire`.

Recipient inbox addressing and recipient X25519 selection SHALL be resolved from the persisted current v1 roster plus current v1 topic derivation state, not from presence payload contents.

The fixed body field set SHALL be:

- `dial_id`
- `punch_token`
- `candidates`
- `member_credential`

The receiving peer SHALL treat body `member_credential` as an asserted identity that still requires validation.
It SHALL accept that credential only when:

- inner `sender_peer_id` matches the credential-derived `peer_id`
- inner `sender_ed25519` matches `member_credential.subject_ed25519_pub`
- the credential matches the trusted roster entry for that `peer_id`
- the credential verifies under the network authority Ed25519 public key

#### Scenario: Candidate exchange stays within the fixed two-message flow
- **WHEN** two enrolled peers coordinate a current v1 punch attempt
- **THEN** they exchange only `dial_offer` and `dial_answer`
- **AND** those messages carry only the fixed body fields

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

### Requirement: Presence is advisory for dial target selection only
The system MAY use current v1 presence to decide whether a peer appears online.

The system SHALL NOT use presence as the authority for remote `MemberCredential`, remote X25519 identity, or inbox topic derivation.

For the default current v1 discover path, this capability SHALL consume only `DiscoverPeer.online_state` from `miopunch-poc-v1-presence-discover` as its presence-owned input surface.

Presence-only observations that stay outside `DiscoverView.peers[]` SHALL NOT become dial targets through this capability.

#### Scenario: Presence does not supply trusted recipient identity
- **WHEN** a current v1 dialer sees a peer as `online` in Discover
- **THEN** it still resolves the remote credential and inbox topic from the persisted trusted roster and topic scope
- **AND** it does not treat presence as sufficient proof of dial recipient identity

#### Scenario: Unknown presence-only peer does not become a dial target
- **WHEN** a runtime observes presence for a `peer_id` that is absent from the trusted current v1 `DiscoverView`
- **THEN** current v1 dial/punch does not treat that observation as a discover-owned dial target
- **AND** trust resolution still requires the persisted roster

### Requirement: Attempt scheduling is bounded by the fixed 5B matrix
The system SHALL schedule at most 4 concurrent candidate-pair attempts and SHALL enforce a fixed 10 second total budget.

The system SHALL stop at the first successful selected path.

Each candidate-pair attempt SHALL record whether it selected `direct_ipv4`, selected `punching_ipv4`, timed out, was canceled, or failed.

#### Scenario: Attempt budget is bounded and explainable
- **WHEN** a current v1 punch run starts
- **THEN** the runtime attempts candidate pairs within the fixed 5B bounds
- **AND** it records evidence for attempted pairs, timeouts, selected path, and the selected result

#### Scenario: Attempt evidence records failure modes and convergence
- **WHEN** multiple current v1 candidate-pair attempts run in parallel
- **THEN** each failed attempt records `timeout`, `canceled`, or `failed`
- **AND** once one attempt selects the path, the remaining attempts converge and stop

### Requirement: PathResult is the only output of this capability
The system SHALL output `PathResult` from current v1 dial/punch.

`PathResult` SHALL contain only:

- the selected UDP path
- resource ownership needed by the next layer
- the trusted remote identity handoff (`peer_id` and `MemberCredential`)
- punch evidence, including the selected UDP path kind

`PathResult` SHALL be a closed handoff for `miopunch-poc-v1-secure-session`; that next layer SHALL NOT need to reopen roster lookup or dial target selection to recover remote identity.
The system SHALL NOT embed KCP/TLS/yamux or session-recipe selection into `PathResult`.

#### Scenario: Session upgrade remains outside dial/punch
- **WHEN** current v1 dial/punch succeeds
- **THEN** the result is a `PathResult`
- **AND** the session layer still chooses how to upgrade that path without re-reading roster authority
- **AND** the remote identity in that result has already passed sender, roster, and authority validation

#### Scenario: PathResult identifies the selected UDP path kind
- **WHEN** current v1 dial/punch succeeds through UDP direct reachability
- **THEN** the resulting punch evidence identifies `direct_ipv4` as the selected path
- **AND** when it succeeds through UDP punching, the resulting punch evidence identifies `punching_ipv4` as the selected path
