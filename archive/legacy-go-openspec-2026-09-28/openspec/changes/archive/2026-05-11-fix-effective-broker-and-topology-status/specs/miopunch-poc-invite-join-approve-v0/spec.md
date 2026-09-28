## ADDED Requirements

### Requirement: Owner/admin selects current effective brokers from explicit or built-in candidates
The system SHALL derive `brokers_effective` from exactly one candidate source
when an owner/admin node creates or maintains current network broker state:

- if explicit `local.mqtt_broker` configuration exists, use only that
  configured list;
- otherwise, use the built-in broker candidate list.

The system SHALL normalize, de-duplicate, and probe reachability for those
candidates, then keep at most two reachable endpoints in source order as the
current `brokers_effective` pair.

#### Scenario: Explicit local broker configuration wins over built-in defaults
- **WHEN** an owner/admin node has explicit
  `local.mqtt_broker=["broker-a:1883","broker-b:1883","broker-c:1883"]`
- **THEN** it selects `brokers_effective` only from that explicit list
- **AND** it does not mix in built-in broker candidates

#### Scenario: Built-in broker defaults are used only when explicit config is absent
- **WHEN** an owner/admin node has no explicit `local.mqtt_broker` configuration
- **THEN** it derives `brokers_effective` from the built-in broker candidate
  list
- **AND** the resulting effective broker set contains at most two reachable
  endpoints

### Requirement: Membership applies the full effective broker set to persisted peer signaling state
When a membership bundle includes `brokers_effective`, the system SHALL persist
the full effective broker set as the post-join MQTT signaling broker state for
that net.

On successful `join`, the joiner SHALL save its local `mqtt_broker` state from
the membership bundle's `brokers_effective` list before future runtime
signaling.

On successful `approve`, the approver SHALL save the joiner's peer config from
that same full `brokers_effective` list before that config is used for future
`ping` or `sh` signaling.

#### Scenario: Joiner listens on the effective broker set after join
- **WHEN** a joiner receives a valid `membership_bundle` with
  `brokers_effective=["broker-a:1883","broker-b:1883"]`
- **THEN** the joiner's saved local state uses that effective broker set
- **AND** subsequent acceptor signaling uses the same primary/secondary pair

#### Scenario: Approver dials joiner through the effective broker set
- **WHEN** an approver accepts a join request from a joiner whose seed peer
  advertised a different broker before membership
- **THEN** the approver saves the joiner's peer config with the net's effective
  broker set
- **AND** subsequent peer tasks from the approver to that joiner use the same
  primary/secondary broker pair
