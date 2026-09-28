## 1. OpenSpec

- [x] 1.1 Add proposal, design, and delta specs for governance bootstrap/admin access.
- [x] 1.2 Validate the change with strict OpenSpec validation.

## 2. Backend Governance State

- [x] 2.1 Add local governance classification and capability helpers.
- [x] 2.2 Add `init_network` task args, task dispatch, bootstrap mode, and confirmed create-new mode.
- [x] 2.3 Ensure create-new preserves identity/runtime preferences but resets net, governance head, decls, known peers, bootstrap evidence, and invite approval cache.
- [x] 2.4 Add admin preflight to invite, approve, and approve-decision paths.
- [x] 2.5 Reject `invite` mode `auto` with `NOT_IMPLEMENTED`.

## 3. LocalAPI, CLI, and Desktop Runtime

- [x] 3.1 Add `init_network` to supported LocalAPI task kinds and client/bridge usage.
- [x] 3.2 Add CLI command support for bootstrap and create-new network initialization.
- [x] 3.3 Expose governance capabilities in desktop state and diagnostics.
- [x] 3.4 Update desktop frontend Settings/Admin gating to use daemon capabilities.

## 4. Tests and Validation

- [x] 4.1 Add focused Go tests for governance classification, init-network modes, admin preflight, and auto-mode rejection.
- [x] 4.2 Add LocalAPI tests for `init_network` and desktop governance capabilities.
- [x] 4.3 Add browser tests for Settings bootstrap, create-new flow, and non-admin invite gating.
- [x] 4.4 Run focused Go tests for touched packages.
- [x] 4.5 Run focused frontend browser tests.
- [x] 4.6 Run real Windows/WSL2 validation before committing.
