## ADDED Requirements

### Requirement: Traversal demux decisions are trace-diagnosable
The UDP traversal demux SHALL provide trace-level diagnostics for packet routing decisions that affect direct handshake and punching attempts.

Diagnostics SHALL avoid logging plaintext credentials, private keys, encrypted payload bytes, or full sensitive message bodies.

The trace surface SHALL make these cases distinguishable:

- tagged packet received
- traversal message decode failure
- missing or unknown transaction ID
- packet routed to an endpoint
- endpoint queue full or packet dropped
- best-effort auto-response to an unknown transaction request

#### Scenario: Unknown transaction packet is diagnosable
- **WHEN** the demux receives a valid tagged traversal request for a transaction ID with no open endpoint
- **THEN** the demux emits trace diagnostics identifying the packet as an unknown transaction
- **AND** if it sends a best-effort response, that response decision is also trace-diagnosable

#### Scenario: Routed traversal packet is diagnosable
- **WHEN** the demux receives a valid tagged traversal packet for an open endpoint
- **THEN** the demux emits trace diagnostics identifying that the packet was routed
- **AND** the endpoint receive loop can continue without direct reads from the underlying UDP socket

#### Scenario: Decode failures do not leak payloads
- **WHEN** the demux receives a tagged traversal packet that cannot be decoded
- **THEN** the demux emits trace diagnostics for the decode failure
- **AND** it does not log encrypted payload bytes or secret material
