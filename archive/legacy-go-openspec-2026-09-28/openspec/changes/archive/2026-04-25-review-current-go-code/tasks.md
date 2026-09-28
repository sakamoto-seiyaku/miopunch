## 1. Baseline and Guardrails

- [x] 1.1 Record the review baseline commit and confirm the working tree state.
- [x] 1.2 Re-read the review-only boundary: produce findings only, do not patch code.
- [x] 1.3 Confirm applicable skills for apply: `$dev`, `$go-code-review`, `$go-concurrency`, and relevant `go-*` review companions.

## 2. Automated Review Inputs

- [x] 2.1 Run `export PATH=/usr/local/go/bin:$PATH`.
- [x] 2.2 Run `gofmt -d .` and record whether formatting diffs exist without writing them back.
- [x] 2.3 Run `go test ./...` and record failures or flakes as findings candidates.
- [x] 2.4 Run `go vet ./...` and record diagnostics as findings candidates.
- [x] 2.5 Run `bash scripts/check_no_xtcp_imports.sh` and record naming/import violations as findings candidates.

## 3. Manual Go Code Review

- [x] 3.1 Review core networking packages: `connectivity/`, `dataplane/`, `internal/punching/`, `internal/coordinator/`, and `internal/wire/`.
- [x] 3.2 Review service/API packages: `internal/signaling/`, `internal/localapi/`, `internal/http_panel/`, `internal/controlplane/`, and `internal/task/`.
- [x] 3.3 Review command/tool entrypoints: `cmd/`, `tools/`, `stun/`, and `nat/`.
- [x] 3.4 Review tests for meaningful assertions, failure messages, concurrency safety, and coverage gaps around recently added behavior.

## 4. Findings Report

- [x] 4.1 Create `openspec/changes/review-current-go-code/findings.md` using Must Fix / Should Fix / Nits grouping.
- [x] 4.2 Ensure every finding has severity, `file:line`, rule category, description, and impact.
- [x] 4.3 Re-read each finding against the referenced code and remove anything that cannot be justified.
- [x] 4.4 Verify no Go code, tests, scripts, or runtime files were modified by the review.
