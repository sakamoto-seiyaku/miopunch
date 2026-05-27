## 1. Runtime Authority

- [ ] 1.1 Restore the extracted-v1 product build graph so `cmd/miopunch`, `internal/localapi`, and `internal/pocv1/...` no longer require missing legacy authority packages.
- [ ] 1.2 Add `internal/pocv1/runtime` and implement the fixed six-stage runtime model plus the `SecureSession -> Shell` ping gate.
- [ ] 1.3 Define `UserSummary`, structured `Evidence`, and the final user-facing `reason_code` mapping in the runtime authority layer.
- [ ] 1.4 Own peer-session and shell-session lifecycle in `internal/pocv1/runtime` instead of legacy task internals.

## 2. LocalAPI v1

- [ ] 2.1 Keep the `localapi` shell but replace the governing extracted-v1 contract with `JSON-RPC` over Unix socket / named pipe.
- [ ] 2.2 Add runtime events and shell attach as dedicated stream channels alongside the RPC control plane.
- [ ] 2.3 Expose the product actions through a shared `action + args` model and keep `/api/v0/desktop/state` plus legacy task routes out of the governing extracted-v1 contract.

## 3. CLI Wiring

- [ ] 3.1 Rewire `cmd/miopunch` product verbs to consume the v1 runtime/`localapi` RPC path while preserving `--format json`, `--report`, and `--redact` for non-interactive commands.
- [ ] 3.2 Make `sh` auto-progress the missing stages and only attach after a successful identity-bound `ping` or `hello`.
- [ ] 3.3 Keep `up` as the explicit daemon command and add same-user daemon auto-bootstrap for CLI callers when the shared daemon is unreachable.
- [ ] 3.4 Reuse shell transport only as plumbing, not as the owner of stage or gate authority.

## 4. Acceptance

- [ ] 4.1 Add focused tests for stage progression, summary/evidence output, failure mapping, `sh` gate enforcement, and v1 runtime DTO/API stability.
- [ ] 4.2 Add required Linux two-node smokes covering `up -> init-network/invite/approve/join -> ls -> ping -> sh ls -> sh -> revoke`.
- [ ] 4.3 Verify failure paths still export `stage`, `reason_code`, `facts`, and `suggestions` in CLI JSON/report surfaces.
- [ ] 4.4 Verify the product package graph resolves without missing legacy authority imports.
- [ ] 4.5 Record Windows/Linux real-machine interoperability as post-06x follow-up scope, not as a blocker for the Linux CLI gate.
