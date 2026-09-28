## Context

MNT-03 currently has useful fresh-start stages for 2, 3, 4, 6, and 12-node validation. The public `mnt03-smoke` and `mnt03-selftest` gates call those stages separately, so the early network formation work is repeated instead of validating one network that grows over time.

The product behavior under test is a single network that starts blank, gains members, preserves control-plane state, changes neighbor shape as membership grows, and then survives perturbations.

## Goals / Non-Goals

**Goals:**

- Make public MNT-03 gates run one progressive network growth flow by default.
- Preserve fresh-start stages as explicit debug entry points.
- Keep artifacts machine-readable and checkpointed so failures identify the growth point.
- Keep lab fixtures as infrastructure only; do not inject product semantic state.

**Non-Goals:**

- Do not remove existing fresh-start stage functions.
- Do not change product bootstrap, topology, or neighbor-selection logic unless the progressive gate exposes a defect.
- Do not require cross-gate state reuse between separate `mnt03-smoke`, `mnt03-selftest`, and `mnt03-fulltest` invocations.

## Decisions

- Public gates call a new progressive stage with a `--until-checkpoint` target. This keeps CI invocations independent while eliminating repeated work within each gate.
- The progressive flow creates the complete NAT fixture once, starts only the needed nodes as the network grows, and keeps MQTT/STUN/captures alive for the run.
- Checkpoints are recorded inside one run under checkpoint-specific artifact directories. Existing aggregate summaries continue to report pass/fail counts at the gate level.
- The node order remains capability-oriented: `n01 -> n03 -> n04 -> n05 -> n06 -> n07 -> n08 -> n09 -> n10 -> n11 -> n02 -> n12`.
- Existing fresh-start stages remain callable with `--stage 2node-substrate`, `--stage 3node-bootstrap`, `--stage 4node-reachability`, `--stage 6node-bootstrap-more`, and `--stage 12node-full`.

## Risks / Trade-offs

- Progressive runs can make later failures depend on earlier state. Mitigation: preserve debug fresh-start stages and checkpoint artifacts.
- Starting nodes incrementally against a full NAT fixture is more complex than starting all nodes at once. Mitigation: add small helpers for starting/installing selected nodes and reuse existing join/assert helpers.
- Checkpoint artifact layout changes for public gates. Mitigation: keep summary files stable and include checkpoint paths in proof JSON.
