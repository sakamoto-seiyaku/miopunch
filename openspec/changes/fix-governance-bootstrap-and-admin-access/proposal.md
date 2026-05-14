## Why

The current invite/approve path can let a non-admin local identity create an
approval declaration that is later rejected during hello as `issuer not admin`.
Desktop first-run handling also exposes an Owner/Admin mode as frontend-only
state, so users have no authoritative way to bootstrap a new network or recover
from stale local governance state.

## What Changes

- Add an explicit local network initialization task for blank-node owner/admin
  bootstrap and confirmed creation of a new local network.
- Classify local governance state and expose admin capabilities through desktop
  runtime state.
- Require owner/admin capability before `invite`, `approve`, and
  `approve_decision` can issue or publish membership material.
- Reject `invite --mode auto` with `NOT_IMPLEMENTED` until true auto-approval is
  designed.
- Update Desktop Settings/Admin flows to use daemon-provided governance
  capabilities instead of frontend-only first-run Owner/Admin state.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-poc-governance-snapshot-v0`: Defines local governance lifecycle
  classification and explicit bootstrap/create-new network semantics.
- `miopunch-poc-invite-join-approve-v0`: Requires local admin preflight for
  invite/approve flows and rejects unsupported auto mode.
- `miopunch-poc-localapi-v0`: Adds the `init_network` task and desktop
  governance capability state.
- `miopunch-desktop-gui-v0`: Replaces frontend-only first-run Owner/Admin mode
  with daemon-backed initialization and create-new-network flows.

## Impact

- Affected daemon task code:
  - invite, approve, approve decision, and new init-network handling
  - governance state inspection and new-network state reset
- Affected LocalAPI and Wails bridge:
  - supported task kinds
  - desktop state/config JSON shape
  - `CreateTask("init_network", ...)`
- Affected desktop frontend:
  - Settings bootstrap/create-new actions
  - Admin visibility and invite/approval gating
- Validation impact:
  - Focused Go and frontend tests are required during implementation.
  - Full `$dev` validation is required before this code-affecting change enters
    mainline.
