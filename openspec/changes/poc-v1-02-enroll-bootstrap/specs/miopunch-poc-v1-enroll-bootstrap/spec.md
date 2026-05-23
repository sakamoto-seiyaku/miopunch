# miopunch-poc-v1-enroll-bootstrap Specification

## Purpose
Defines the POC v1 trust bootstrap: InviteCapability (MPINV1), join_request, approve, and enroll_response.

## ADDED Requirements

### Requirement: InviteCapability is entry-ticket only
The system SHALL encode InviteCapability as MPINV1 and SHALL NOT embed runtime state such as seed peers, topology, or long-term transport secrets.

### Requirement: join_request uses peer_e2e_v1 to authority
The system SHALL encrypt join_request to `authority_x25519_pub` using peer_e2e_v1.

### Requirement: enroll_response delivers MemberCredential and mailbox_secret
After approval, the system SHALL deliver `MemberCredential + mailbox_secret` to the joiner via peer_e2e_v1 on joiner-provided reply_topic.
