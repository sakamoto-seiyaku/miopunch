## Why

The product path still reuses `internal/coordinator` as the named home for NAT/punching analysis even after `miopunch-lab coord` became the lab-only service entrypoint. This keeps the historical `coord` service mental model attached to MQTT and future signaling backends, which conflicts with the roadmap item to make Door 3 signaling backend work service-neutral.

## What Changes

- Extract the NAT/punching decision logic currently exposed through `coordinator.AnalyzeOnce` into a neutral decision module that can be called by MQTT leader code, future mailbox/overlay signaling, and the lab coord adapter.
- Keep `miopunch-lab coord` and its `coord` CLI/YAML flags as lab/regression compatibility surface; do not remove the lab service.
- Remove product/POC imports of `internal/coordinator` so product paths depend on a signaling-agnostic decision boundary instead of a lab service package.
- Update OpenSpec wording so MQTT and TCP punching requirements describe a decision engine / decision boundary rather than “coordinator” as the product semantics.
- Preserve the existing `NatHoleResp` wire shape, gather snapshot model, attempt behavior, and lab regression expectations.

## Capabilities

### New Capabilities

- `miopunch-punching-decision`: Defines the service-neutral NAT/punching decision boundary that turns exchanged peer snapshots into attempt-ready `NatHoleResp` outputs.

### Modified Capabilities

- `miopunch-mqtt-signaling`: Replace the MQTT requirement’s “coordinator analysis boundary” wording with the neutral punching decision boundary.
- `miopunch-tcp-p2p-v0`: Replace TCP derivation requirements that name the coordinator with signaling-agnostic punching decision behavior.

## Impact

- Affected code:
  - Future implementation should add `internal/punchdecision` (or equivalent neutral module) and move analysis helpers/tests out of `internal/coordinator`.
  - Product/POC callers such as MQTT visitor leader and POC dialing should call the neutral module.
  - `cmd/miopunch-lab` and `internal/coordinator` remain as the lab coord service adapter.
- Affected specs:
  - New `miopunch-punching-decision` spec.
  - Delta updates for `miopunch-mqtt-signaling` and `miopunch-tcp-p2p-v0`.
- Compatibility:
  - No wire-format changes.
  - No CLI flag removals.
  - No runtime default signaling change.
  - No lab topology or regression baseline changes.
