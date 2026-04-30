## ADDED Requirements

### Requirement: Presence is delivered as signed control-plane state
After a node joins a net, it SHALL publish signed presence state to active neighbors and to bootstrap/recovery contacts when no active neighbor is available.

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

### Requirement: MNT-03 control-plane messages remain broker-only metadata
MQTT SHALL remain a control-plane, signaling, and mailbox path for MNT-03.

The system MUST NOT use MQTT as a data-plane relay for payload exchange, shell bytes, or active-edge keepalive payloads.

#### Scenario: Broker artifacts contain no data-plane payload
- **WHEN** an MNT-03 active edge exchanges a recognizable payload
- **THEN** broker logs and packet captures do not contain that data-plane payload
