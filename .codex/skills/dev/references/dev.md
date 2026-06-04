# Miopunch Dev — References

This folder contains extra notes that are helpful during development but are not always needed in-context.

## Test Gates (manual)

Use this full set when a code-affecting change is committed or merged into mainline. Docs-only / notes-only / OpenSpec-only changes do not require this full set unless explicitly requested.

Host checks:

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
```

Lab checks (QEMU VM):

```bash
./lab/host/labctl nat-profile-selftest
```

Artifacts are rsynced to `lab/_artifacts/`.

Historical/debug suites such as `xtcp-*`, `poc-e2e-*`, `mnt01-*`, `mnt02-*`,
and `mnt03-*` are not current required gates.

## Lab Troubleshooting

- **Go not found on host**: run `export PATH=/usr/local/go/bin:$PATH` (or call `/usr/local/go/bin/go`).
- **SSH config stale** (e.g. `LAB_SSH_PORT` changed): regenerate:

  ```bash
  rm -f lab/_state/ssh_config
  ./lab/host/labctl wait
  ```

## Windows/WSL2 Debugging

- Confirm daemon ownership before interpreting results. Stop stale Windows or Linux daemons and verify the CLI reaches the intended LocalAPI endpoint.
- Prefer foreground Linux daemon sessions for long experiments; background shells can hide daemon exits.
- When starting a Windows daemon from WSL2, avoid TTY launch paths and verify readiness with a Windows CLI command such as `ls`.
- Run CLI probes with `--format json --report <path>` and preserve the report artifact.
- Record at least: task id, peer id, net id, `reason_code`, `attempt_path`, `path_family`, `data_proto`, `hello`, `ping`, and whether `session_reused=true`.
- Treat `session_reused=true` as evidence for reuse behavior, not proof that a fresh TCP/UDP path works.
- Separate link-layer failures from hello/governance failures; examples like `issuer is not an admin` are not punching failures.
- Temporary fault binaries are allowed for proof, but do not commit fault code. Restore both daemons to non-fault current binaries after validation.
- If the user explicitly defers a gate for a live debug batch, record the narrowed validation scope and the deferred gate in the notes.

## Notes

- Prefer adding new lab cases for new behavior; avoid mutating baseline NAT/P0 scenarios unless explicitly required.
- When transport (not punching) is the focus, prefer a known-good punching baseline (e.g. `core-01`) and validate event sequences via `lab/guest/cases/expect/*.events.json`.
- Hot-path diagnostics should default to debug-level logging. Use info-level logs for lifecycle events or operator-actionable state, not candidate/target dumps.
