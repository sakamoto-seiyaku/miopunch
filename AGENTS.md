# miopunch agent rules

- Before making any code/spec/test changes, read and follow the `$dev` skill at `.codex/skills/dev`.
- When Go work touches goroutines, channels, mutexes, WaitGroups, shared state, or parallel execution, explicitly read and follow the `$go-concurrency` skill in addition to `$dev`.
- Prefer the OpenSpec workflow for non-trivial work: propose/new-change → apply → verify → archive.
- Full validation is required when a code-affecting change enters the mainline branch, whether by direct commit or merge.
- Code-affecting changes include Go code, tests, lab/runtime scripts, and other execution-affecting files.
- Full validation for mainline code-affecting changes:
  - `export PATH=/usr/local/go/bin:$PATH`
  - `go test ./...`, `go vet ./...`, `bash scripts/check_no_xtcp_imports.sh`
  - Temporarily do not run VM lab gates until POC v1 mainline specs and validation scope are updated.
  <!-- - `./lab/host/labctl selftest`, `./lab/host/labctl xtcp-selftest`, `./lab/host/labctl xtcp-connectivity-selftest`, `./lab/host/labctl xtcp-fulltest` -->
- Docs-only / notes-only / OpenSpec-only changes do not require the full validation set unless explicitly requested.
- Prefer `miopunch` naming; avoid introducing new `xtcp` names/paths/imports.
