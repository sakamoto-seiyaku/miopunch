## ADDED Requirements

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

