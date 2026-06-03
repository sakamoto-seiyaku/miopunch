# miopunch-poc-v1-dial-punch Specification

## Purpose
定义当前 POC v1 的 UDP direct-first dial/punch：`dial_offer`、`dial_answer`、bounded attempt runtime 与 `PathResult`。

## Requirements

### Requirement: Current v1 path establishment uses UDP direct-first with punching fallback
The system SHALL establish current POC v1 peer-to-peer paths using UDP carrier semantics only.

The system SHALL gather UDP path material for the runtime-owned UDP socket, including direct addresses, assisted addresses, and ordinary STUN mapped address samples from configured STUN servers or the archived built-in STUN endpoint set.

The system SHALL derive attempt-ready UDP `NatHoleResp` values through the service-neutral punching decision boundary using the exchanged snapshots.

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

#### Scenario: NAT decision drives UDP punching behavior
- **WHEN** two current v1 peers exchange mapped and assisted UDP snapshots
- **THEN** the answering peer derives visitor and client `NatHoleResp` outputs through the punching decision boundary
- **AND** each peer attempts UDP punching using its assigned mode, role, TTL, candidate ports, random send count, and random listen count

#### Scenario: mode2 and mode4 random listening is executable
- **WHEN** the punching decision selects a UDP mode that requests `ListenRandomPorts`
- **THEN** the receiver opens bounded temporary UDP listen sockets for that attempt
- **AND** the selected winner socket is handed to the next layer
- **AND** non-winning temporary sockets are closed

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

#### Scenario: Path establishment remains UDP-only
- **WHEN** a current v1 peer tries to establish a direct path
- **THEN** it uses only UDP direct reachability and UDP punching
- **AND** it does not use TCP, relay, QUIC, or another carrier inside this change

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

#### Scenario: Candidate exchange carries UDP decision material
- **WHEN** two enrolled peers coordinate a current v1 punch attempt
- **THEN** they exchange only `dial_offer` and `dial_answer`
- **AND** those messages carry the UDP snapshot and answer-side decision outputs needed for UDP path establishment

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

#### Scenario: Region-specific STUN arbitration is not required
- **WHEN** current v1 dial/punch builds the UDP snapshot
- **THEN** it may include ordinary STUN mapped address samples
- **AND** it does not require CN/global STUN view arbitration to derive the UDP decision

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

### Requirement: Android local candidates avoid standard interface enumeration
Current v1 UDP path establishment SHALL gather Android app local direct candidates without calling Go standard `net.Interfaces()` or `net.InterfaceAddrs()` on Android builds.

Android local candidate gathering SHALL attempt an Android-safe address provider that does not require MAC addresses, `RTM_GETLINK`, or a bound `NETLINK_ROUTE` socket.

If Android local candidate gathering returns no usable direct addresses, the system SHALL report that condition in trace diagnostics and SHALL NOT silently publish `127.0.0.1` as a peer-reachable direct candidate.

#### Scenario: Android app sandbox blocks standard enumeration
- **WHEN** an Android build gathers current v1 local candidates
- **THEN** it does not invoke Go standard interface enumeration
- **AND** it records Android provider diagnostics for the candidate source used

#### Scenario: Android provider has no usable address
- **WHEN** Android local candidate gathering returns no peer-reachable address
- **THEN** the candidate set excludes loopback fallback addresses
- **AND** trace diagnostics identify that no usable Android local candidate was found

### Requirement: Route-source derivation supplements peer direct candidates
Current v1 UDP path establishment SHALL be able to derive local host candidates from known peer direct targets by using the kernel-selected UDP source address for those targets.

The derived candidate SHALL use the runtime UDP socket port for the matching IP family and SHALL be subject to the same IP-family policy filtering as other current v1 direct candidates.

Derived candidates SHALL supplement, not replace, enumerated local candidates.

#### Scenario: Peer target reveals Android local source address
- **WHEN** Android receives a peer direct target that is route-reachable
- **THEN** the runtime derives the local source IP selected for that target
- **AND** it adds that local source IP with the runtime UDP port as a host candidate

#### Scenario: Route-source derivation fails for a target
- **WHEN** a peer direct target cannot be dialed for route-source derivation
- **THEN** the runtime records trace diagnostics for that target
- **AND** it continues with the remaining candidate sources

### Requirement: Presence is advisory for dial target selection only
The system SHALL treat current v1 presence as advisory and MAY use it to decide whether a peer appears online.

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
