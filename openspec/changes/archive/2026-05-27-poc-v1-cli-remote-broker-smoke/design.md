## Context

The existing `poc-e2e` lab path is heavier than the requested CLI pre-gate: it assumes the old containerized broker setup, exercises `sh attach`, and includes revoke-boundary coverage that is useful for deeper validation but expensive for a frequent branch gate.

This change adds a lighter smoke path for the product CLI. It still validates the real product flow end to end, but it does so in one VM with two Docker node containers and a shared remote MQTT broker supplied from the host environment.

## Goals / Non-Goals

**Goals:**
- Add a cheap, repeatable pre-gate for the CLI product path.
- Cover `up -> init-network -> invite -> approve -> join -> ls -> ping -> sh ls` only.
- Require an explicit broker endpoint and fail fast when it is missing.
- Keep the existing `poc-e2e` smoke/selftest/fulltest gates unchanged.

**Non-Goals:**
- Do not reintroduce the archived large test matrix.
- Do not add `sh attach` or revoke checks to this gate.
- Do not require two VMs or VM-local broker setup.
- Do not change the deeper runtime acceptance contract beyond what is needed to support the smoke.

## Decisions

### 1) Use a dedicated smoke runner instead of reusing the heavier `poc-e2e` entrypoint

The new gate is a separate lab command and guest runner. This keeps the cheaper pre-gate isolated from the historical `poc-e2e` matrix and makes its acceptance boundary explicit.

Alternatives considered:
- Reuse `poc-e2e` with a mode flag. Rejected because it would preserve too much legacy coupling and make the cheap gate harder to reason about.
- Add the smoke as a subset inside the existing fulltest. Rejected because the intent is to have a low-cost pre-gate with its own contract.

### 2) Run `miopunch up` directly inside each container

The smoke does not depend on `install-system-daemon` or system service ownership. Each node container launches `miopunch up` in the background with an explicit `--localapi`, `--state_path`, and `--broker`.

Alternatives considered:
- Continue using the systemd-based service path. Rejected because it adds unnecessary state and makes the broker override harder to propagate cleanly.
- Teach the service wrapper to inherit the broker override. Rejected because it couples this new gate to service installation behavior that is out of scope.

### 3) Add an explicit runtime broker override

`miopunch up` needs a way to persist a remote broker endpoint instead of always starting an embedded broker during `init-network`. The runtime stores the override in its metadata and uses it as the broker source of truth when present.

Alternatives considered:
- Infer the broker from environment only. Rejected because the runtime must remain deterministic after startup.
- Add a separate hidden debug command. Rejected because the smoke should use the same product CLI surface that real users exercise.

### 4) Keep the broker requirement host-driven

The broker URL is supplied via host env and the guest runner fails fast if it is empty. This makes the gate explicit and avoids silently falling back to embedded broker behavior.

## Risks / Trade-offs

- [Risk] The remote broker may be reachable from the host but not from the VM network. → Mitigation: fail fast with a clear broker-reachability error during the smoke rather than masking it behind later CLI failures.
- [Risk] Direct daemon startup could diverge from the normal service path. → Mitigation: keep the smoke narrow and continue to validate service-oriented paths in the heavier lab gates.
- [Risk] A shared broker introduces cross-run interference if multiple smoketests reuse the same topics. → Mitigation: keep the smoke isolated by run and network state, and prefer dedicated broker credentials or namespaces when the CI wiring is added.

## Migration Plan

1. Land runtime broker override support and `up --broker`.
2. Add the guest smoke runner and `labctl` entry.
3. Wire the smoke into CI with a required host broker URL.
4. Keep the existing `poc-e2e` gates intact as the deeper validation layer.

## Open Questions

- Should the new smoke run in a dedicated workflow or be attached to the existing lab gate workflow?
- Should the broker URL be a repository secret, an environment variable, or both?
- Should the smoke artifacts be retained with the same retention policy as the heavier lab gates?
