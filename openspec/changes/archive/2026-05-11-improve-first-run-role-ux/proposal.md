## Why

Portable first-run nodes currently report `self.role=unknown` until the user
creates or joins a network. The desktop UI treats `unknown` as non-admin, so a
fresh extracted bundle hides the create-invite and approve-entry points even
though create-invite is the intended way to bootstrap a new network.

## What Changes

- Treat an uninitialized first-run topology as an owner candidate in the desktop
  UI only.
- Keep daemon startup and backend state lazy: first run SHALL NOT create
  `net.json`, governance, or decl state until the user starts an operation.
- Keep broker/runtime state lazy as well: owner-candidate visibility SHALL NOT
  imply that `brokers_effective` already exists before `invite/create` or
  successful `join`.
- Preserve current role transitions:
  - create invite initializes a new network and makes the local node the genesis
    owner/admin through the existing backend flow.
  - join accepts membership from an existing network and leaves the local node a
    member.
- Keep owner/admin-only controls hidden for real member nodes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: the desktop GUI SHALL expose new-network and
  join-network entry points on a blank first-run node.

## Impact

- Desktop static frontend role gating.
- Desktop Playwright smoke coverage.
- OpenSpec desktop GUI delta spec for first-run role UX.
- Broker selection semantics remain defined elsewhere; this change only governs
  UI visibility and first-run role wording.
