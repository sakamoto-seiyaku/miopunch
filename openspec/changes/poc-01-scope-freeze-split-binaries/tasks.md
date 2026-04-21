## 1. Pre-flight Validation

- [x] 1.1 Run baseline `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 1.2 Run baseline `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 1.3 Run baseline `bash scripts/check_no_xtcp_imports.sh`
- [x] 1.4 Confirm `gofmt -l .` is clean

## 2. Create `miopunch-lab` Binary (Lab Entrypoint)

- [x] 2.1 Add new `cmd/miopunch-lab` binary that exposes existing lab subcommands: `coord/peer/stun/mqtt-broker`
- [x] 2.2 Update usage/help output to consistently use the binary name `miopunch-lab`
- [x] 2.3 Ensure `go build ./cmd/miopunch-lab` succeeds on host

## 3. Reserve `miopunch` for Product/POC (Remove Lab Commands)

- [x] 3.1 Remove `coord/peer/stun/mqtt-broker` subcommands from `cmd/miopunch`
- [x] 3.2 Add explicit guidance when users attempt `miopunch coord|peer|stun|mqtt-broker` (tell them to use `miopunch-lab ...`)
- [x] 3.3 Ensure `go build ./cmd/miopunch` succeeds and `miopunch --help` is product-oriented

## 4. Update Lab Automation to Use `miopunch-lab`

- [x] 4.1 Update `lab/host/labctl` to build `./cmd/miopunch-lab` and push the `miopunch-lab` binary into the VM
- [x] 4.2 Update `lab/guest/bin/*` scripts to reference the `miopunch-lab` binary path/name by default
- [x] 4.3 Update `lab/README.md` and any referenced runbooks/reports to use `miopunch-lab` commands
- [x] 4.4 Add a repo-wide grep check that no scripts/docs still instruct `miopunch coord|peer|stun|mqtt-broker`

## 5. Tests (Entrypoints & Help/Flags)

- [x] 5.1 Refactor entrypoints to allow testing help/arg parsing without `os.Exit` (e.g. `run(args) (exitCode int)` pattern)
- [x] 5.2 Add unit tests covering `miopunch` help output and the “lab commands moved” guidance
- [x] 5.3 Add unit tests covering `miopunch-lab` help output for at least one subcommand (e.g. `coord --help`)

## 6. Post-change Validation

- [x] 6.1 Re-run `export PATH=/usr/local/go/bin:$PATH && go test ./...`
- [x] 6.2 Re-run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`
- [x] 6.3 Re-run `bash scripts/check_no_xtcp_imports.sh`
- [x] 6.4 Run lab minimum smoke: `./lab/host/labctl selftest`
- [x] 6.5 If entering mainline: run full lab gate set (`./lab/host/labctl xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`)
