## Why

Current POC v1 dial/punch diverged from the intended archived UDP punching core:
it exchanges only simple candidates and locally synthesizes a mode0 response, so
it cannot run NAT/STUN-based mode0..4 decisions or explain hard NAT behavior.

This change restores the archived UDP-only XTCP-style path establishment while
preserving the current POC v1 roster, peer E2E messaging, runtime UDP ownership,
and `PathResult` handoff.

## What Changes

- Replace candidate-only punch coordination with UDP gather snapshots containing
  direct, mapped, and assisted addresses.
- Restore the service-neutral `punchdecision` flow for UDP mode0..4 decision
  generation.
- Rewire POC v1 path establishment to consume attempt-ready `NatHoleResp`
  outputs and use direct-first `connectivity.Attempt`.
- Restore UDP `ListenRandomPorts` execution so mode2/mode4 decisions can run
  instead of only appearing in diagnostics.
- Remove the POC v1 dependence on locally synthesized mode0 pair responses as
  the main path.
- Keep TCP Door-2, TCP punching, relay, carrier negotiation, and CN/global STUN
  arbitration out of this change.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-v1-dial-punch`: changes current POC v1 dial/punch from
  simplified UDP direct-first with mode0 fallback to UDP-only XTCP-style gather,
  decision, attempt, and evidence.

## Impact

- Affects `internal/pocv1/punch`, `internal/pocv1/runtime`,
  `connectivity`, and `internal/punching`.
- Extends the current v1 `dial_offer` / `dial_answer` body shape.
- Adds focused tests for snapshot exchange, decision handoff, direct fallback,
  mode behavior, and UDP random listen cleanup.
- Requires full Go and lab validation before entering mainline because this is a
  code-affecting change to path establishment.
