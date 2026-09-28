## Why

The current MNT-03 public gates rebuild the same early network stages multiple times, which makes the default scenario slower and less representative of a real network growing over time.

MNT-03 should validate one network that starts blank and grows through successive checkpoints until it reaches the full 12-node NAT composite topology.

## What Changes

- Change the default MNT-03 public gates to run a progressive single-network growth flow.
- Keep existing fresh-start stage execution available for manual debugging.
- Emit checkpoint artifacts inside one run instead of treating each small stage as a separate default case.
- Update MNT-03 charter/spec language to distinguish default progressive gates from archived/debug fresh-start stages.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-mainline-nat-composite-network-v0`: MNT-03 layered gates now default to a single continuously growing network, while fresh-start stages are retained only as manual debug entry points.

## Impact

- MNT-03 lab guest runner and host gate wrappers.
- MNT-03 artifact layout and gate summary contents.
- `docs/decisions/mainline-network-test-charter.md`.
- `openspec/specs/miopunch-mainline-nat-composite-network-v0/spec.md`.
