# miopunch-desktop-gui-v0 Specification Delta

## ADDED Requirements

### Requirement: desktop diagnostics preserve join failure facts

The desktop bridge and diagnostics export SHALL preserve runtime action failure facts for `invite`, `approve`, and `join` so operators can inspect broker/topic diagnostics without reproducing the failure in CLI first.

#### Scenario: join failure facts remain available to desktop diagnostics

- **WHEN** a desktop-triggered `join` action fails with broker/topic facts
- **THEN** the returned bridge/runtime error exposes those facts to desktop consumers
- **AND** diagnostics export preserves them in `runtime-snapshot.json` or `connection.json` as applicable

#### Scenario: approve failure facts remain available to desktop diagnostics

- **WHEN** a desktop-triggered `approve` action fails with invite signaling facts
- **THEN** the bridge result carries the returned facts without adding a new bridge contract
