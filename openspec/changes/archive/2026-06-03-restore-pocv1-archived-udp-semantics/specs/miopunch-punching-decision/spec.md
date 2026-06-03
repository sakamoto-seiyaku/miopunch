## MODIFIED Requirements

### Requirement: MQTT task path uses daemon-lifetime success-only analyzer memory
The MQTT/task product path SHALL use a daemon-lifetime analyzer for punching mode/index recommendations.

Analyzer memory SHALL be scoped by local peer view, remote peer, and protocol, SHALL have bounded lifetime, and SHALL record only successful mode/index recommendation state.

When a signaling exchange computes decision outputs on one side and sends those outputs to the other side, each side SHALL still report success into its own local analyzer scope.

A peer SHALL NOT report success using analyzer metadata that is scoped only to the other peer's local view.

#### Scenario: Repeated peer punching can reuse successful mode preference
- **WHEN** a peer pair succeeds with a punching mode/index
- **THEN** later rounds for the same local peer, remote peer, and protocol can prefer that mode/index
- **AND** each later round still gathers and exchanges fresh candidate snapshots

#### Scenario: Initiator records success in initiator-local scope
- **WHEN** an initiator receives decision output from a responder-led exchange
- **AND** the initiator succeeds with UDP punching
- **THEN** it records success in the initiator daemon's local remote-peer/protocol analyzer scope
- **AND** it does not write success into the responder daemon's analyzer scope

#### Scenario: Responder records success in responder-local scope
- **WHEN** a responder computes decision output and succeeds with UDP punching
- **THEN** it records success in the responder daemon's local remote-peer/protocol analyzer scope
- **AND** the initiator's success memory remains independent

## ADDED Requirements

### Requirement: Analyzer metadata is reproducible from exchanged UDP decision material
The punching decision boundary SHALL provide enough metadata or deterministic recomputation inputs for each peer to derive its local analyzer success key from the exchanged UDP decision material.

This metadata SHALL NOT require re-running STUN discovery or changing the selected path after a punch attempt succeeds.

#### Scenario: Local analyzer key can be derived after success
- **WHEN** a peer completes UDP punching using a decision-derived `NatHoleResp`
- **THEN** it can derive the local analyzer key needed to report the successful mode/index
- **AND** derivation uses already exchanged decision material rather than fresh network probing
