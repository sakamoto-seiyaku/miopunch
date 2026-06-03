# miopunch-lab-event-expectations Specification

## Purpose
TBD - created by archiving change p35-lab-event-expectations-fixups. Update Purpose after archive.
## Requirements
### Requirement: Lab `--disable-stun` Explicitly Disables STUN
The lab runner SHALL treat `--disable-stun` as an explicit STUN disable signal.
When `--disable-stun` is set, the lab run SHALL disable both user-configured STUN and internal STUN defaults for the session.

#### Scenario: `--disable-stun` produces `gather.stun.skip` evidence
- **WHEN** `mlab-xtcp-run` is invoked with `--disable-stun`
- **THEN** the peers are started with an explicit empty STUN configuration
- **AND** the visitor event stream contains `stage=gather kind=info name=gather.stun.skip`

### Requirement: Derived Connectivity Cases Require Payload Evidence
For any derived case that expects a successful run, the lab validator SHALL require explicit payload evidence in the visitor event stream.

#### Scenario: Success requires `transport.payload_exchanged`
- **WHEN** a derived connectivity case expects `success`
- **THEN** the ordered evidence chain includes a `stage=transport kind=ok name=transport.payload_exchanged` event

