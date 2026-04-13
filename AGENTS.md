# miopunch agent rules

- Before making any code/spec/test changes, read and follow the `$dev` skill at `.codex/skills/dev`.
- Prefer the OpenSpec workflow for non-trivial work: propose/new-change → apply → verify → archive.
- Required validation before commit/archive:
  - `export PATH=/usr/local/go/bin:$PATH`
  - `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`
  - `./lab/host/labctl selftest`, `./lab/host/labctl xtcp-selftest`, `./lab/host/labctl xtcp-connectivity-selftest`, `./lab/host/labctl xtcp-fulltest`
- Prefer `miopunch` naming; avoid introducing new `xtcp` names/paths/imports.
