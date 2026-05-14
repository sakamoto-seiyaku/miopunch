## Context

`pocstate.EnsureGovernanceHeadSnapshot` creates a genesis owner/admin head when
no local head exists. That is valid for first-run bootstrap, but today
`invite`, `approve`, and `approve_decision` use it inside operational flows.
When local state is stale or the current identity is not the admin recorded in
the existing head, the task can still proceed far enough to emit invalid
membership material. The remote side then rejects the connection during hello.

The desktop GUI has a related shortcut: Settings can enable Owner/Admin mode on
an empty node by changing frontend state only. That avoids a first-run dead end
visually, but it does not create a real network, head snapshot, or durable
capability.

## Goals / Non-Goals

**Goals:**

- Make blank-node bootstrap an explicit daemon-backed action.
- Give stale/non-admin local state a clear confirmed path to create a new local
  network without self-promoting inside the old network.
- Fail non-admin invite/approve paths locally before they publish unusable
  approval declarations.
- Expose governance capabilities once through desktop runtime state so GUI
  gating does not duplicate trust logic.
- Keep hello/governance validation as the final defense.

**Non-Goals:**

- Do not implement true `invite --mode auto`.
- Do not migrate members from an old local network into a newly created network.
- Do not add governance proposal/sign/apply flows.
- Do not add identity import or admin key switching.

## Decisions

### Add a daemon task for local network initialization

Add task kind `init_network` with `mode="bootstrap"` and `mode="create_new"`.
`bootstrap` succeeds only when no local net, no governance head, no decls head,
and no members are present. It creates/ensures identity, creates a new net,
creates a genesis governance head with the current identity as owner/admin, and
ensures an empty decl set.

`create_new` is available when the operator explicitly confirms
`confirm="create-new-network"`. It keeps the local identity and runtime settings,
but replaces local network/governance membership state with a new net, genesis
head, empty decls, and empty network peer/bootstrap/invite state.

### Centralize local governance classification

Add shared task-layer classification that reports:

- `no_network`
- `admin_network`
- `member_network`
- `foreign_or_stale_network`

The classifier derives capabilities from local net/head/decls/topology evidence
and current identity. Desktop state exposes this as non-secret governance
capability data.

### Make admin preflight explicit

Operational invite/approve code must load an existing governance head and verify
`head.IsAdmin(selfID.PeerID)` before emitting invite codes, listening for
approvals, or publishing approval decisions. The bootstrap task is the only path
that intentionally creates a genesis head for the current identity.

### Preserve create-new as a local reset, not a promotion

Create-new generates a new `net_id` and governance head. It does not add the
current identity as admin to the old network and does not preserve old member
declarations. This avoids the chicken-and-egg problem without weakening the
trust root of an existing network.

## Risks / Trade-offs

- `create_new` is intentionally destructive for local membership state. The UI
  and CLI require explicit confirmation and report the old/new net IDs.
- Desktop frontend is currently committed as static built assets. Update the
  committed static JS and its browser fixtures/tests in the existing style.
- Some existing tests assume invite implicitly bootstraps governance. Update
  those tests to run `init_network` first where they model an admin network.

## Migration Plan

1. Add OpenSpec deltas and focused tests for the new governance lifecycle.
2. Implement task-layer initialization, classification, and admin preflight.
3. Expose LocalAPI/desktop state contract changes and bridge them through the
   desktop UI.
4. Update CLI and frontend tests.
5. Run focused validation locally; run real Windows/WSL2 validation before
   committing this fix.
