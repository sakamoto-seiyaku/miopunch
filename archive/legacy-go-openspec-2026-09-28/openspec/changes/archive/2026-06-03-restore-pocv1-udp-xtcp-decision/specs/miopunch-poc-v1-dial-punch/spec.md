## MODIFIED Requirements

### Requirement: Current v1 path establishment uses UDP direct-first with punching fallback
The system SHALL establish current POC v1 peer-to-peer paths using UDP carrier
semantics only.

The system SHALL gather UDP/IPv4 path material for the runtime-owned UDP socket,
including direct addresses, assisted addresses, and ordinary STUN mapped address
samples from configured STUN servers or the archived built-in STUN endpoint set.

The system SHALL derive attempt-ready UDP `NatHoleResp` values through the
service-neutral punching decision boundary using the exchanged snapshots.

For reachable direct UDP candidates, the system SHALL attempt UDP direct
reachability before UDP punching.

If UDP direct reachability succeeds, the system SHALL select that UDP path and
SHALL NOT run UDP punching for that direct winner.

If UDP/IPv4 direct reachability fails, the system SHALL fall back to UDP punching
using the derived mode0..4 detect behavior within the bounded attempt budget.

The system SHALL NOT branch to TCP, relay, QUIC, or other carrier types in this
capability.

#### Scenario: Same-LAN host candidates use UDP direct first
- **WHEN** a current v1 peer tries to establish a path to an IPv4 host candidate that is directly reachable over UDP
- **THEN** it selects the UDP direct path
- **AND** it records the selected path as `direct_ipv4`
- **AND** it does not require UDP punching to converge for that direct winner

#### Scenario: Direct timeout falls back to UDP punching
- **WHEN** a current v1 peer tries UDP direct reachability for a UDP peer candidate
- **AND** the direct attempt times out
- **THEN** it attempts UDP punching for the same exchange
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

#### Scenario: Path establishment remains UDP-only
- **WHEN** a current v1 peer tries to establish a direct path
- **THEN** it uses only UDP direct reachability and UDP punching
- **AND** it does not use TCP, relay, QUIC, or another carrier inside this change

### Requirement: Current v1 candidate exchange uses only dial_offer and dial_answer
The system SHALL exchange punch parameters using only `dial_offer` and
`dial_answer` sent over the recipient's roster-backed inbox topic via
`miopunch-poc-v1-controlplane-wire`.

Recipient inbox addressing and recipient X25519 selection SHALL be resolved from
the persisted current v1 roster plus current v1 topic derivation state, not from
presence payload contents.

The body field set SHALL include:

- `dial_id`
- `punch_token`
- `candidates`
- `udp_snapshot`
- `member_credential`

`dial_answer` SHALL additionally include a `udp_decision` field whose JSON body
contains the decision outputs needed by both peers to attempt the path:

- `local_response`
- `remote_response`
- `decision_mode`
- `decision_index`

The receiving peer SHALL treat body `member_credential` as an asserted identity
that still requires validation.
It SHALL accept that credential only when:

- inner `sender_peer_id` matches the credential-derived `peer_id`
- inner `sender_ed25519` matches `member_credential.subject_ed25519_pub`
- the credential matches the trusted roster entry for that `peer_id`
- the credential verifies under the network authority Ed25519 public key

#### Scenario: Candidate exchange carries UDP decision material
- **WHEN** two enrolled peers coordinate a current v1 punch attempt
- **THEN** they exchange only `dial_offer` and `dial_answer`
- **AND** those messages carry the UDP snapshot and answer-side decision outputs needed for UDP path establishment

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
