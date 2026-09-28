## 1. Pre-flight Validation

- [x] 1.1 Run `gofmt -l internal/coordinator/*.go` and ensure clean
- [x] 1.2 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 1.3 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 1.4 Run `bash scripts/check_no_xtcp_imports.sh`

## 2. Coordinator Error Messages

- [x] 2.1 Replace user-visible coordinator error strings containing `xtcp` with `miopunch`/neutral wording (`proxy`/`peer`)
- [x] 2.2 Ensure all related error paths remain semantically equivalent (missing proxy / not allowed / auth failed)

## 3. Post-change Validation

- [x] 3.1 Re-run `go test ./...` + `go vet ./...` (and gofmt)
