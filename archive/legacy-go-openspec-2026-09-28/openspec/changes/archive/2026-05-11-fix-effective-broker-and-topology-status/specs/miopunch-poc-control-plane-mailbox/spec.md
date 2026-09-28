## ADDED Requirements

### Requirement: invite_brokers are selected separately from runtime effective brokers
Invite code broker endpoints SHALL come from the same broker candidate source as
runtime broker selection:

- if explicit `local.mqtt_broker` configuration exists, select only from that
  configured candidate list;
- otherwise, select from the built-in broker candidate list.

When the current runtime `brokers_effective` pair already occupies one or two
reachable endpoints, invite generation SHOULD prefer other reachable endpoints
from the same candidate source. If too few alternatives exist, the system MAY
reuse the current effective brokers rather than fail invite generation.

#### Scenario: Invite prefers broker endpoints outside the current runtime pair
- **WHEN** an owner/admin node already has `brokers_effective=["broker-a:1883","broker-b:1883"]`
- **AND** its configured candidate list also contains reachable
  `broker-c:1883`
- **THEN** invite generation prefers `broker-c:1883` for
  `invite_brokers`
- **AND** join and approve still use only the `invite_brokers` carried in the
  code

#### Scenario: Invite may fall back to the current runtime pair
- **WHEN** an owner/admin node has no reachable invite candidates outside its
  current `brokers_effective`
- **THEN** invite generation may reuse the current effective brokers
- **AND** invite generation does not fail solely because separate invite brokers
  are unavailable

### Requirement: Joined peer signaling uses the net effective broker set
The system SHALL use the net's pinned effective broker set for peer seed
configs and runtime control-plane tasks after a node has joined a net, rather
than each node's pre-join default broker or invite-only brokers.

The POC v0 minimum behavior SHALL carry up to two endpoints in
`brokers_effective`, treat the first endpoint as primary and the second as
secondary, and use that pair for local acceptor listening and saved peer
configs.

#### Scenario: Post-join signaling avoids broker split
- **WHEN** two nodes join the same net through an invite whose membership bundle
  carries `brokers_effective=["broker-a:1883","broker-b:1883"]`
- **AND** either node had a different default broker before joining
- **THEN** both nodes' post-join peer signaling state uses that effective broker
  set
- **AND** later `ping`, `sh`, and related control-plane tasks use the same
  primary/secondary broker pair

#### Scenario: Runtime signaling may use the secondary effective broker
- **WHEN** a joined node cannot use `brokers_effective[0]` for a runtime
  signaling attempt
- **AND** `brokers_effective[1]` exists
- **THEN** the runtime signaling path may attempt the secondary effective broker
- **AND** the node does not fall back to its pre-join default broker
