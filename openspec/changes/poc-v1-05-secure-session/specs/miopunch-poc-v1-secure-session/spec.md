# miopunch-poc-v1-secure-session Specification

## Purpose
Defines the POC v1 data-plane secure session recipe and identity pinning rules.

## ADDED Requirements

### Requirement: POC v1 recipe is UDP + KCP + TLS1.3 + yamux
The system SHALL establish POC v1 peer sessions using UDP carrier, KCP transport, a TLS 1.3 secure channel, and yamux multiplexing.

### Requirement: TLS is pinned to MemberCredential identity
The system SHALL pin the TLS peer identity to the peer's MemberCredential: the peer certificate Ed25519 public key MUST match `MemberCredential.subject_ed25519_pub`, and the credential MUST verify under the network authority key.
