# miopunch-stun-probe-v0 Specification

## Purpose
TBD - created by archiving change door-2-stun-module-probe. Update Purpose after archive.
## Requirements
### Requirement: STUN Probe Command Produces Per-Endpoint Evidence (UDP + TCP)
The system SHALL provide a `miopunch-lab stun probe` command that probes STUN endpoints over both UDP and TCP and emits machine-readable evidence.
The probe command SHALL accept STUN endpoints in the same string forms used by the rest of the system:
`host:port` (dual), `udp://host:port` (UDP-only), `tcp://host:port` (TCP-only).

#### Scenario: Probe produces one JSONL record per endpoint
- **WHEN** the operator runs `miopunch-lab stun probe --stun <endpoint1,endpoint2,...>`
- **THEN** the command emits JSONL to stdout
- **AND** the output contains one JSON object per input endpoint

### Requirement: Probe Output Includes Classification For Updating Built-in Lists
For each endpoint, the probe output SHALL include a classification decision that can be used to update built-in STUN lists:
`dual`, `udp_only`, `tcp_only`, or `remove`.

#### Scenario: dual is selected when both protocols meet the threshold
- **WHEN** an endpoint achieves `udp_ok_count >= ok_threshold` and `tcp_ok_count >= ok_threshold`
- **THEN** the probe output includes `decision: "dual"`

#### Scenario: remove is selected when neither protocol meets the threshold
- **WHEN** an endpoint achieves `udp_ok_count < ok_threshold` and `tcp_ok_count < ok_threshold`
- **THEN** the probe output includes `decision: "remove"`

### Requirement: Probe Output Has Stable Minimal Fields
Each JSONL record SHALL include at least the following fields:
`endpoint`, `udp_ok_count`, `tcp_ok_count`, `udp_rtt_ms_min`, `tcp_rtt_ms_min`, `udp_mapped_addrs`, `tcp_mapped_addrs`, `decision`.
The system MAY include additional fields.

#### Scenario: Record includes minimal required fields
- **WHEN** `miopunch-lab stun probe` emits a record for an endpoint
- **THEN** the JSON object includes the required minimal fields
- **AND** `udp_mapped_addrs` and `tcp_mapped_addrs` are JSON arrays (possibly empty)

### Requirement: Probe Parameters Are Configurable With Defaults
The probe command SHALL support configuring the number of attempts and the ok-threshold:
- `--attempts` (default: `3`)
- `--ok-threshold` (default: `2`)

#### Scenario: Defaults are used when flags are omitted
- **WHEN** the operator runs `miopunch-lab stun probe` without specifying `--attempts` and `--ok-threshold`
- **THEN** the probe uses `attempts=3`
- **AND** the probe uses `ok_threshold=2`

