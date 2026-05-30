## 1. CLI and runtime contract

- [x] 1.1 Extend `sh ls` argument parsing and runtime `ShellArgs` to support `--ready` and reject `--ready` when a concrete target is also supplied.
- [x] 1.2 Extend shell control request/response types so shell-list requests can ask for ready probing and replies can carry per-target readiness status details.
- [x] 1.3 Update current v1 `doShellList` / remote shell-list handling so default `sh ls <peer>` remains unchanged while `sh ls <peer> --ready` returns only ready targets plus structured readiness evidence.

## 2. Target readiness probing

- [x] 2.1 Add a shelltarget readiness-probe path for Linux `local` and Windows `wsl:*` targets that confirms tmux preflight without requiring an existing tmux session.
- [x] 2.2 Add a bounded non-interactive readiness probe for Windows `ssh:*` targets using explicit SSH options that avoid password prompts and host-key side effects.
- [x] 2.3 Classify ready-probe outcomes into `ready`, `unsupported`, and `unknown`, and map those classifications into `sh ls --ready` facts/report/data without failing the whole command on partial probe failures.

## 3. Verification

- [x] 3.1 Add focused CLI/runtime tests for default `sh ls`, `sh ls --ready`, and invalid `sh ls <peer> <target> --ready` combinations.
- [x] 3.2 Add shelltarget-focused tests for ready probing across tmux-missing, no-server-but-tmux-installed, and bounded SSH probe failure cases.
- [x] 3.3 Run the relevant focused test set for the touched shell/runtime packages and confirm the new ready-probe behavior matches the spec.
