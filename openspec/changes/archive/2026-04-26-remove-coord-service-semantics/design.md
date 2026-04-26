## Context

The product binary already no longer exposes lab commands such as `coord`, `peer`, `stun`, or `mqtt-broker`; those remain under `miopunch-lab`. However, product and POC flows still reuse `internal/coordinator.AnalyzeOnce` for MQTT-led exchange and POC dialing, so the analysis/decision logic is still named and layered as a `coord` service concern.

The roadmap calls out this cleanup as Door 3 preparation: the lab coord server should remain an experiment/regression entrypoint, while NAT and punching decisions should be a reusable module shared by MQTT, future mailbox/overlay signaling, and lab coord.

## Goals / Non-Goals

**Goals:**

- Introduce a neutral punching decision boundary independent of any signaling backend or service process.
- Move product/POC callers away from importing `internal/coordinator`.
- Keep lab coord behavior available by making it an adapter over the neutral decision engine.
- Preserve existing `NatHoleResp` output semantics, TCP/UDP decision behavior, and lab expectations.
- Update specs and docs so future work says “decision engine/boundary” instead of using coordinator as product architecture language.

**Non-Goals:**

- Do not remove `miopunch-lab coord`, `--coord`, or lab YAML `coord` fields.
- Do not change wire messages, event shapes, or `NatHoleResp` field names.
- Do not redesign signaling backends, introduce backend plugins, or change MQTT session/barrier behavior.
- Do not change TCP/UDP punching algorithms, defaults, or NAT lab baselines.

## Decisions

### 1) Create `internal/punchdecision` as the neutral module

**Decision:** Extract the code that turns `wire.NatHoleVisitor` + `wire.NatHoleClient` snapshots into visitor/client `wire.NatHoleResp` values into `internal/punchdecision`.

The package should own:
- NAT feature classification orchestration around `nat` and `internal/punching`.
- STUN view arbitration for UDP and TCP observations.
- TCP `+100` candidate derivation, `tcp_punching_enabled`, `tcp_punching_error`, and detect behavior derivation.
- Existing analyzer scoring state and cleanup behavior.
- A convenience `AnalyzeOnce` entrypoint for single-shot callers.

**Why:** This keeps the shared decision model close to the punching domain, not to the lab coord service shape.

**Alternative considered:** Rename `internal/coordinator` wholesale. Rejected because the lab coord server is still a real compatibility surface and should remain easy to identify.

### 2) Keep `internal/coordinator` as the lab service adapter

**Decision:** `internal/coordinator` should keep service/session responsibilities: control-plane hello handling, auth/precheck, SID generation, client/visitor matching, response delivery, success reports, and lab logs. It should call `internal/punchdecision` for decisions.

**Why:** This minimizes behavior risk and preserves the lab command while removing product coupling to coord semantics.

**Alternative considered:** Move all server code into `cmd/miopunch-lab`. Rejected because the current internal package already encapsulates lab server behavior and has tests.

### 3) Preserve API shape at the wire boundary

**Decision:** The extracted decision boundary continues to accept `*wire.NatHoleVisitor` and `*wire.NatHoleClient`, and returns the same two `*wire.NatHoleResp` values plus error. Any stateful engine type should be internal to the module or exposed only as needed by the lab adapter.

**Why:** Keeping the wire-facing contract stable avoids migration work in `connectivity.Attempt`, MQTT signaling, dataplane, and lab scripts.

**Alternative considered:** Introduce a new decision input/output DTO. Rejected for this cleanup because it would be a larger semantic refactor without immediate value.

### 4) Move tests with the behavior

**Decision:** Tests that assert decision behavior should move with the extracted package. Adapter tests that assert lab coord server behavior should remain under `internal/coordinator`.

**Why:** Future changes to analysis logic should not need to reference a lab service package.

## Risks / Trade-offs

- **Import churn causing cycles** → Keep `internal/punchdecision` below service packages; it may import `connectivity`, `nat`, `internal/punching`, `internal/wire`, and `internal/logutil`, but it must not import `internal/coordinator` or `internal/peer`.
- **Lost analyzer state in lab coord** → Preserve a stateful engine for the lab adapter instead of forcing every call through a stateless helper.
- **Spec/docs wording drift** → Add a focused grep/task check for product paths importing `internal/coordinator` and for new product-facing “coordinator derives” wording.
- **Over-cleaning lab semantics** → Explicitly keep `miopunch-lab coord` and lab-only `coord` flags in scope.

## Migration Plan

- Add `internal/punchdecision` by moving existing decision code and tests with minimal edits.
- Update product/POC callers to use `punchdecision.AnalyzeOnce`.
- Update `internal/coordinator` to construct/use a `punchdecision.Engine` while preserving its public service behavior.
- Run focused tests for `internal/punchdecision`, `internal/coordinator`, `internal/peer`, and `internal/task`, then run host gates for the code-affecting apply.

## Open Questions

- None for this cleanup; deeper signaling backend plugin design remains separate Door 3 work.
