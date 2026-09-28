## Why

Current POC v1 validation intentionally retired the historical XTCP/MNT VM gates, but it also left the old P0 NAT profile selftest without a current, explicit role. The baseline `core-01..10` NAT lab remains useful as a substrate sanity check, so it should be restored under a precise name that matches what it tests.

## What Changes

- Add a current VM validation gate named `nat-profile-selftest`.
- **BREAKING**: remove the generic `labctl selftest` command from current lab entrypoints instead of keeping it as an alias.
- Define `nat-profile-selftest` as the VM/netns NAT profile substrate check that runs `core-01..10` and expects `pass=10 fail=0`.
- Keep old `xtcp-*`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`, and `mnt03-*` commands historical/debug-only, not current required gates.
- Update developer guidance and gate automation so current validation runs host checks plus `nat-profile-selftest`.

## Capabilities

### New Capabilities
- `miopunch-vm-nat-profile-selftest`: Current VM/netns NAT profile substrate validation for `core-01..10`.

### Modified Capabilities
- `miopunch-poc-v1-current-mainline`: Current mainline validation includes `nat-profile-selftest` as the only required VM gate.

## Impact

- `lab/host/labctl` command dispatch and help text.
- Guest runner name under `lab/guest/bin/`.
- Current validation guidance in `AGENTS.md`, `$dev` skill docs, `lab/README.md`, `openspec/project.md`, and related docs.
- Test gate automation in `.codex/skills/dev/scripts/run_test_gates.sh`.
