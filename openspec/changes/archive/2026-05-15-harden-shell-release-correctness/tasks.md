## 1. OpenSpec

- [x] 1.1 Validate the OpenSpec change after proposal, design, specs, and tasks are complete.

## 2. Windows WSL tmux Session Discovery

- [x] 2.1 Add a focused parser for default `tmux list-sessions` output that extracts clean session names before the first colon.
- [x] 2.2 Use the default-output parser only on the Windows `wsl:<distro>` session listing path.
- [x] 2.3 Add focused tests covering default tmux output, empty/no-server output, duplicate names, and already-clean session names.

## 3. Windows SSH tmux Commands

- [x] 3.1 Factor Windows SSH tmux list/preflight command argument construction into testable helpers.
- [x] 3.2 Remove the remote `--` token from Windows `ssh:<host>` tmux list and tmux preflight commands while keeping attach behavior unchanged.
- [x] 3.3 Add focused tests proving SSH list/preflight commands invoke remote `tmux` directly after the host.

## 4. Shell Attach Readiness

- [x] 4.1 Replace the fixed attach-ready sleep with deterministic bridge setup and immediate non-blocking setup failure polling.
- [x] 4.2 Preserve existing post-ready behavior where nil backend wait sends `shell_exit ok=true` and non-nil backend wait reports shell-layer failure.
- [x] 4.3 Add focused tests for pre-ready backend failure and post-ready normal backend completion.

## 5. Invite Broker Helper Boundary

- [x] 5.1 Rename or document the IP-canonicalizing invite broker helper so it is not used for emitted invite-code brokers.
- [x] 5.2 Ensure invite-code broker selection continues to normalize, validate, de-duplicate, probe, and emit the selected hostname endpoint unchanged.
- [x] 5.3 Add or adjust focused tests that fail if a reachable hostname broker is converted to an A-record IP in invite-code output.

## 6. Validation

- [x] 6.1 Run focused Go tests for touched packages.
- [x] 6.2 Run `openspec validate harden-shell-release-correctness --strict`.
- [x] 6.3 Record any validation not run and why.

Validation note:

- Focused validation run: `go test ./internal/shelltarget ./internal/pocacceptor ./internal/task ./internal/controlplane -count=1`.
- OpenSpec validation run: `openspec validate harden-shell-release-correctness --strict`.
- Host validation run: `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`, and `git diff --check && git diff --cached --check`.
- Full `$dev` lab gates were not run in this step because no mainline commit/merge or archive was requested; run them before this code-affecting change enters mainline or is archived.
