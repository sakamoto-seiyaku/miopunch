# miopunch-poc-control-plane-mesh-first-fallback Specification

## Purpose
`miopunch-poc-control-plane-mesh-first-fallback` defines the POC v0 delivery policy for control-plane request/response messages: mesh-first delivery with MQTT mailbox fallback, and the deduplication behavior required to prevent dual-path delivery from causing duplicate side effects.

## ADDED Requirements

### Requirement: Request delivery is mesh-first with MQTT fallback
When a node has at least one mesh neighbor available, it SHALL attempt to deliver control-plane request/response messages via mesh first.
If no valid response is observed within 1 second, it SHALL publish the same ciphertext payload to the destination peer's MQTT inbox topic as a fallback.

#### Scenario: Mesh-first delivery succeeds without requiring MQTT fallback
- **WHEN** a sender has a mesh path to the destination and sends a request
- **THEN** the destination receives the request via mesh and produces a valid response
- **AND** the request is considered successful without requiring MQTT fallback

#### Scenario: MQTT fallback delivers when mesh delivery does not respond in time
- **WHEN** a sender sends a request via mesh but observes no response within 1 second
- **THEN** it publishes the request ciphertext to the destination inbox topic via MQTT
- **AND** the destination can receive and respond via the MQTT mailbox path

### Requirement: Dual-path delivery does not cause duplicate side effects
If a receiver observes the same message via multiple delivery paths (e.g., both mesh and MQTT), it SHALL apply at most one set of side effects for that message ID.

#### Scenario: Receiver processes duplicate deliveries at most once
- **WHEN** a receiver receives the same `msg_id` via mesh and MQTT within the dedup window
- **THEN** it processes the message at most once

### Requirement: LAN smoke is reproducible with three processes
The system SHALL provide a reproducible LAN smoke harness that can be executed as three separate processes on the same LAN segment:
- node A (sender)
- node B (forwarder)
- node C (receiver)

The harness SHALL demonstrate:
- bounded flooding (H=3) forwarding via B from A to C
- mesh-first request delivery from A to C
- MQTT fallback without duplicate side effects

#### Scenario: Three-process LAN smoke validates mesh-first and MQTT fallback
- **WHEN** the three nodes are started with a neighbor topology A↔B↔C and a reachable MQTT broker
- **THEN** a request from A to C can complete successfully
- **AND** duplicate side effects from dual-path delivery do not occur
