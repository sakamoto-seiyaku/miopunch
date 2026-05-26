# miopunch-poc-v1-dial-punch Specification

## Purpose
定义当前 POC v1 的 UDP-only dial/punch：`dial_offer`、`dial_answer`、bounded attempt runtime 与 `PathResult`。

## ADDED Requirements

### Requirement: Current v1 path establishment uses UDP punching only
The system SHALL establish current POC v1 peer-to-peer paths using UDP punching only.

The system SHALL NOT branch to TCP, relay, QUIC, or other carrier types in this capability.

#### Scenario: Path establishment stays on UDP only
- **WHEN** a current v1 peer tries to establish a direct path
- **THEN** it performs UDP punching only
- **AND** it does not change carriers inside this change

### Requirement: Current v1 candidate exchange uses only dial_offer and dial_answer
The system SHALL exchange punch parameters using only `dial_offer` and `dial_answer` sent over the recipient's roster-backed inbox topic via `miopunch-poc-v1-controlplane-wire`.

Recipient inbox addressing and recipient X25519 selection SHALL be resolved from the persisted current v1 roster plus current v1 topic derivation state, not from presence payload contents.

The fixed body field set SHALL be:

- `dial_id`
- `punch_token`
- `candidates`
- `member_credential`

#### Scenario: Candidate exchange stays within the fixed two-message flow
- **WHEN** two enrolled peers coordinate a current v1 punch attempt
- **THEN** they exchange only `dial_offer` and `dial_answer`
- **AND** those messages carry only the fixed body fields

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

#### Scenario: Attempt budget is bounded and explainable
- **WHEN** a current v1 punch run starts
- **THEN** the runtime attempts candidate pairs within the fixed 5B bounds
- **AND** it records evidence for attempted pairs, timeouts, and the selected result

### Requirement: PathResult is the only output of this capability
The system SHALL output `PathResult` from current v1 dial/punch.

`PathResult` SHALL contain only:

- the selected UDP path
- resource ownership needed by the next layer
- the trusted remote identity handoff (`peer_id` and `MemberCredential`)
- punch evidence

`PathResult` SHALL be a closed handoff for `miopunch-poc-v1-secure-session`; that next layer SHALL NOT need to reopen roster lookup or dial target selection to recover remote identity.
The system SHALL NOT embed KCP/TLS/yamux or session-recipe selection into `PathResult`.

#### Scenario: Session upgrade remains outside dial/punch
- **WHEN** current v1 dial/punch succeeds
- **THEN** the result is a `PathResult`
- **AND** the session layer still chooses how to upgrade that path without re-reading roster authority
