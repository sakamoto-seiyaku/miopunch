# miopunch-poc-v1-enroll-bootstrap Specification

## Purpose
Defines the POC v1 trust bootstrap: InviteCapability (MPINV1), join_request, approve, and enroll_response.

## ADDED Requirements

### Requirement: InviteCapability is entry-ticket only
The system SHALL encode InviteCapability as MPINV1 and SHALL NOT embed runtime state such as seed peers, topology, or long-term transport secrets.

#### Scenario: InviteCapability carries only bootstrap fields
- **WHEN** a v1 invite is generated for a new joiner
- **THEN** the payload contains only entry-ticket bootstrap fields such as authority keys, broker route, join topic, invite id, and expiry
- **AND** it does not carry seed peers, topology hints, or transport secrets

### Requirement: join_request uses peer_e2e_v1 to authority
The system SHALL encrypt join_request to `authority_x25519_pub` using peer_e2e_v1.

#### Scenario: Join request is sealed to the authority
- **WHEN** a joiner publishes a v1 `join_request`
- **THEN** the request is encrypted to `authority_x25519_pub` using `peer_e2e_v1`
- **AND** intermediaries can route the message without reading the join body

### Requirement: enroll_response delivers MemberCredential and mailbox_secret
After approval, the system SHALL deliver `MemberCredential + mailbox_secret` to the joiner via peer_e2e_v1 on joiner-provided reply_topic.

#### Scenario: Approval returns the minimal enrollment package
- **WHEN** authority approval succeeds for a v1 join request
- **THEN** the response is published to the joiner-provided `reply_topic`
- **AND** the encrypted body contains `MemberCredential + mailbox_secret`
