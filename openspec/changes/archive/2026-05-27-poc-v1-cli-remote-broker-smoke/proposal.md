## Why

The current `poc-e2e` lab gates are useful but too heavy for a cheap CLI pre-gate. They still assume the old broker/container shape and include `sh attach` and revoke-boundary coverage that should stay in the deeper validation path.

This change adds a lighter pre-gate that exercises the product CLI against a real remote broker with a single VM and two node containers.

## What Changes

- Add a new `labctl` smoke entry for the CLI pre-gate.
- Add a guest runner that validates `up -> init-network -> invite -> approve -> join -> ls -> ping -> sh ls`.
- Make runtime broker selection accept an explicit broker endpoint override for this gate.

## Capabilities

### New Capabilities

- `miopunch-poc-v1-cli-remote-broker-smoke`: lightweight Linux-only CLI smoke gate for the headless product path with a host-supplied remote broker.
- `miopunch-poc-v1-runtime-broker-override`: explicit runtime broker override support for `miopunch up` and `init-network`.

### Modified Capabilities

None.

## Impact

- `lab/host/labctl`
- `lab/guest/lib/poc_v1_cli_smoke.sh`
- `lab/guest/bin/mlab-poc-v1-cli-smoke`
- `cmd/miopunch/up.go`
- `cmd/miopunch/up_windows.go`
- `cmd/miopunch/up_options.go`
- `internal/pocv1/runtime/*`

## Test Plan

- Verify the new smoke fails fast when the broker endpoint is missing.
- Verify the new smoke passes with a reachable remote broker.
- Keep the existing `poc-e2e` smoke/selftest/fulltest unchanged.

## Assumptions

- Remote broker auth is not required for the first pass.
- `sh attach` and revoke-boundary checks stay in the heavier POC e2e gates.
