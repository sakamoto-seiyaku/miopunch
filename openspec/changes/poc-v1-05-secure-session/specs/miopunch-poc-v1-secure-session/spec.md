# miopunch-poc-v1-secure-session Specification

## Purpose
Defines the POC v1 data-plane secure session recipe and identity pinning rules.

## ADDED Requirements

### Requirement: POC v1 recipe is UDP + KCP + TLS1.3 + yamux
The system SHALL establish POC v1 peer sessions using UDP carrier, KCP transport, a TLS 1.3 secure channel, and yamux multiplexing.

#### Scenario: Session upgrade follows the single v1 recipe
- **WHEN** a v1 `PathResult` is handed to the session layer
- **THEN** the runtime upgrades it through KCP, then TLS 1.3, then yamux
- **AND** it does not branch to QUIC, TCP, or other recipes

### Requirement: TLS is pinned to MemberCredential identity
The system SHALL pin the TLS peer identity to the peer's MemberCredential: the peer certificate Ed25519 public key MUST match `MemberCredential.subject_ed25519_pub`, and the credential MUST verify under the network authority key.

#### Scenario: TLS peer identity is accepted only with a matching credential
- **WHEN** a v1 TLS handshake completes for a peer session
- **THEN** the presented Ed25519 certificate key must match the remote `MemberCredential.subject_ed25519_pub`
- **AND** that credential must verify under the network authority key before the session is accepted
