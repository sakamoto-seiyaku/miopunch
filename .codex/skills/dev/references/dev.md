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
./lab/host/labctl selftest
./lab/host/labctl xtcp-selftest
./lab/host/labctl xtcp-connectivity-selftest
./lab/host/labctl xtcp-fulltest
```

Artifacts are rsynced to `lab/_artifacts/`.

## Lab Troubleshooting

- **Go not found on host**: run `export PATH=/usr/local/go/bin:$PATH` (or call `/usr/local/go/bin/go`).
- **SSH config stale** (e.g. `LAB_SSH_PORT` changed): regenerate:

  ```bash
  rm -f lab/_state/ssh_config
  ./lab/host/labctl wait
  ```

## Notes

- Prefer adding new lab cases for new behavior; avoid mutating baseline NAT/P0 scenarios unless explicitly required.
- When transport (not punching) is the focus, prefer a known-good punching baseline (e.g. `core-01`) and validate event sequences via `lab/guest/cases/expect/*.events.json`.
