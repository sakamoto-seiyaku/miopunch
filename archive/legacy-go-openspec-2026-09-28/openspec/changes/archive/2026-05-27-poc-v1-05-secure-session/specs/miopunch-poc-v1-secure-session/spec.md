# miopunch-poc-v1-secure-session Specification

## Purpose
定义当前 POC v1 的 secure session recipe：`PathResult -> PeerSession`，以及固定的 6A identity pin 规则。

## ADDED Requirements

### Requirement: Current v1 secure session consumes a closed PathResult handoff
The system SHALL accept `PathResult` as the only dial or punch handoff into this capability.

`PathResult` SHALL already include the remote `peer_id` and remote `MemberCredential` needed for identity pinning.
The system SHALL NOT reopen roster lookup or punch selection logic inside this capability.

#### Scenario: Session layer does not re-read remote identity inputs
- **WHEN** current v1 dial/punch hands off a successful `PathResult`
- **THEN** the session layer has the UDP path and remote identity material it needs
- **AND** it does not reread roster state before starting the fixed session recipe

### Requirement: Current v1 uses one session recipe only
The system SHALL establish current POC v1 peer sessions using exactly one recipe:

- UDP path from `PathResult`
- KCP transport
- TLS 1.3 secure channel
- yamux multiplexing

The system SHALL NOT branch to QUIC, TCP, or any alternative recipe inside this capability.

#### Scenario: Session upgrade follows the fixed v1 recipe
- **WHEN** a current v1 `PathResult` is handed to the session layer
- **THEN** the runtime upgrades it through KCP, then TLS 1.3, then yamux
- **AND** it does not choose a second recipe

### Requirement: PeerSession is the only upper-layer session boundary
The system SHALL expose `PeerSession` with stream-oriented operations such as `OpenStream` and `AcceptStream` as the upper-layer contract of this capability.

`AcceptStream` SHALL preserve the stream-open envelope, including `kind` and `metadata`, in the returned `AcceptedStream`.
Upper layers SHALL NOT depend on KCP/TLS/yamux internals directly.

#### Scenario: Upper layers consume only PeerSession
- **WHEN** a current v1 shell or ping workflow uses an established session
- **THEN** it interacts with `PeerSession`
- **AND** it does not need to know the underlying recipe details

### Requirement: TLS identity is pinned to MemberCredential
The system SHALL pin the TLS peer identity to the remote `MemberCredential`.

The presented certificate Ed25519 public key MUST match `MemberCredential.subject_ed25519_pub`.
The credential MUST verify under the network authority before the session is accepted.
The remote `MemberCredential` consumed here SHALL come from `PathResult`, not from a second roster lookup reopened by this capability.
The local session certificate SHALL be a self-signed Ed25519 certificate created from the local device key material.

#### Scenario: Session is rejected when credential and certificate disagree
- **WHEN** a TLS handshake completes but the presented Ed25519 key does not match the remote `MemberCredential.subject_ed25519_pub`
- **THEN** the session is rejected
- **AND** no alternate recipe is tried as a fallback
