## 1. OpenSpec

- [x] 1.1 Create proposal, design, and delta specs for real shell regression fixes.
- [x] 1.2 Validate the OpenSpec change after implementation artifacts are complete.

## 2. Invite Broker Selection

- [x] 2.1 Preserve normalized hostname broker endpoints when selecting reachable invite brokers.
- [x] 2.2 Add focused tests proving reachable hostnames stay in invite codes and persisted broker state.

## 3. Shell Target Discovery

- [x] 3.1 Decode UTF-16LE Windows command output for WSL distro discovery while preserving UTF-8 output.
- [x] 3.2 Classify missing `tmux` output as `ErrTmuxMissing` for Linux shell, zsh, and Windows cmd forms.
- [x] 3.3 Treat installed `tmux` with no running server as an empty session list.
- [x] 3.4 Avoid the fragile Windows `wsl.exe` `tmux list-sessions -F "#S"` argument path.
- [x] 3.5 Add focused tests for WSL output decoding, tmux missing text, and no-server text.

## 4. Shell Attach Lifecycle

- [x] 4.1 Add and handle a `shell_exit` control op for normal remote shell completion.
- [x] 4.2 Preserve setup failure semantics when the backend exits before attach readiness.
- [x] 4.3 Treat expected PTY read close errors as close semantics resolved by backend wait.
- [x] 4.4 Add focused tests for `shell_exit` bridge behavior and expected PTY read close classification.

## 5. Validation

- [x] 5.1 Run focused Go tests for touched packages.
- [x] 5.2 Run `openspec status --change fix-real-shell-regressions`.
- [x] 5.3 Record any validation not run and why.

Validation note:

- Focused validation run: `go test ./internal/task ./internal/shelltarget ./internal/shellproto ./internal/pocacceptor -count=1`.
- OpenSpec validation run: `openspec validate fix-real-shell-regressions --strict`.
- Full `$dev` mainline gates were not run in this step because no commit/merge was requested; run them before this code-affecting change enters mainline.
