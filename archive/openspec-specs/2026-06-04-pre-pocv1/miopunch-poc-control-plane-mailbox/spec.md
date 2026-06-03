# miopunch-poc-control-plane-mailbox Specification

## Purpose
`miopunch-poc-control-plane-mailbox` defines legacy POC v0 control-plane delivery via an untrusted MQTT broker mailbox.
It specifies how peer inbox topics are derived from `net_secret` and `peer_id`, and how join codes pin broker endpoints via `invite_brokers`.

Current extracted `poc-v1` uses dedicated capabilities for single-broker bootstrap, persistence-owned topic derivation, and observation-only presence. This spec is no longer the source of truth for the extracted v1 path.

## Requirements
### Requirement: Current extracted v1 treats this capability as legacy/v0 only
The system SHALL treat this capability as the governing mailbox contract only for legacy/v0 paths.

Current extracted `poc-v1` SHALL use `miopunch-poc-v1-enroll-bootstrap`, `miopunch-poc-v1-persistence`, and `miopunch-poc-v1-presence-discover` as the governing contracts for bootstrap broker selection, topic derivation, and presence semantics.

#### Scenario: Extracted v1 implementers do not import mailbox semantics from this legacy spec
- **WHEN** a developer implements or reviews the extracted `poc-v1` path
- **THEN** they treat this mailbox capability as legacy/v0 reference material only
- **AND** they do not use its multi-broker or signed-presence semantics as extracted-v1 source of truth

### Requirement: Peer inbox topics are deterministically derived and non-enumerable
The system SHALL treat an MQTT broker as an untrusted mailbox for control-plane delivery.
To avoid topic enumeration, the system SHALL derive each peer inbox topic as a high-entropy value (≥128 bits effective entropy) from `net_secret` and `peer_id`, without requiring centralized topic allocation.
The derived inbox topic SHALL NOT include `peer_id` in plaintext.

#### Scenario: Inbox topic derivation is deterministic for a given peer
- **WHEN** the system derives the inbox topic using the same `net_secret` and the same canonical `peer_id`
- **THEN** it produces the same `inbox_topic` value
- **AND** the `peer_id` string does not appear as a substring in the topic name

### Requirement: Inbox topics are unique per peer_id within the same net_secret
The system SHALL ensure that different peers do not share the same inbox topic when `peer_id` differs (for the same `net_secret`).

#### Scenario: Different peer_id values produce different inbox topics
- **WHEN** the system derives `inbox_topic` for two different canonical `peer_id` values under the same `net_secret`
- **THEN** the derived `inbox_topic` values are different

### Requirement: Inbox topic derivation algorithm and encoding are fixed for POC v0
For POC v0, the system SHALL derive the inbox topic as follows:
- `net_id_raw16 = sha256(net_secret)[:16]`
- `name16 = HKDF(net_secret, salt=net_id_raw16, info="miopunch/v0/topic.inbox/"+peer_id, L=16)`
- `inbox_topic = base32(raw,no-pad,name16)` and normalize to lower-case when used as an MQTT topic name

The system SHALL include the `peer_id` in the HKDF `info` field.
The system SHALL use `base32(raw,no-pad)` encoding and SHALL NOT use base32 padding for inbox topic names.

#### Scenario: Inbox topic format is lower-case base32(raw,no-pad) with fixed length
- **WHEN** the system derives an inbox topic for any valid canonical `peer_id`
- **THEN** the MQTT topic name contains only characters `a-z` and `2-7`
- **AND** the topic name is lower-case
- **AND** the topic name length is `26` characters

### Requirement: Join code pins broker endpoints via invite_brokers
The system SHALL include `invite_brokers` in the join code to pin the broker instance(s) used for invite/join delivery.
`invite_brokers` SHALL contain `1..2` broker endpoints in `host:port` form.
The join code MUST NOT include broker authentication material (e.g., username/password/certificates).

During `invite/approve/join`, the system SHALL use only the `invite_brokers` from the join code for MQTT subscribe/publish needed for the invite/join exchange.

#### Scenario: Joiner and approver use the same broker endpoint set from join code
- **WHEN** an approver generates a join code
- **THEN** the code includes `invite_brokers` with 1–2 endpoints
- **AND** a joiner using that code attempts MQTT delivery using exactly that `invite_brokers` list

### Requirement: invite_brokers endpoints are canonicalized to deterministic connectable addresses
When writing `invite_brokers` into the join code, the system SHALL attempt to canonicalize broker endpoints to deterministic connectable addresses.
If an endpoint is a hostname, the system SHALL resolve it via the configured resolver and write a single `ip:port` into the join code (using the first returned A record).
If hostname resolution fails, the system SHALL keep the hostname in the join code but MUST emit a strong warning during `invite/approve/join` that join success depends on both sides resolving the hostname to the same broker instance.

#### Scenario: Hostname endpoints are fixed to ip:port or produce a warning
- **WHEN** an invite is generated with a broker endpoint specified as a hostname
- **THEN** the system either writes a deterministic `ip:port` into `invite_brokers`
- **OR** it keeps the hostname and emits a warning explaining the DNS/geo-splitting risk and remediation options

### Requirement: For legacy POC v0, invite_brokers are selected separately from runtime effective brokers
Legacy POC v0 invite code broker endpoints SHALL come from the same broker candidate source as
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

### Requirement: For legacy POC v0, presence is delivered as signed control-plane state
After a legacy POC v0 node joins a net, it SHALL publish signed presence state to active neighbors and to bootstrap/recovery contacts when no active neighbor is available.

Presence SHALL include sender identity, message ID, timestamp, state-head summaries, and reachability hints. Presence SHALL NOT include raw endpoint addresses, secret material, or data-plane payload.

#### Scenario: Presence refreshes online evidence
- **WHEN** a joined node publishes presence to another joined node
- **THEN** the receiver can update last-seen and state-head evidence for that sender
- **AND** the received presence does not contain data-plane payload or secret material

### Requirement: bootstrap_more uses the control-plane mailbox
When a joiner exhausts its initial bootstrap recommendations, it SHALL send a signed `bootstrap_more_request` to its approver or another online admin through the control-plane mailbox.

The request body SHALL include attempted peer IDs and coarse failure summaries. It MUST NOT include IP addresses, ports, private endpoint details, or data-plane payload.

The responder SHALL return a signed `bootstrap_more_response` with up to two de-duplicated candidate peer IDs selected by reachability bucket order.

#### Scenario: bootstrap_more does not leak endpoint details
- **WHEN** a joiner requests more bootstrap candidates
- **THEN** the request includes attempted peer IDs and coarse failure summaries
- **AND** it does not include endpoint addresses or ports
- **AND** the response returns de-duplicated peer candidates

### Requirement: For legacy POC v0, joined peer signaling uses the net effective broker set
The legacy POC v0 system SHALL use the net's pinned effective broker set for peer seed
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

### Requirement: MNT-03 control-plane messages remain broker-only metadata
MQTT SHALL remain a control-plane, signaling, and mailbox path for MNT-03.

The system MUST NOT use MQTT as a data-plane relay for payload exchange, shell bytes, or active-edge keepalive payloads.

#### Scenario: Broker artifacts contain no data-plane payload
- **WHEN** an MNT-03 active edge exchanges a recognizable payload
- **THEN** broker logs and packet captures do not contain that data-plane payload
