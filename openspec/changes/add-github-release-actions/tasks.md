## 1. Build Readiness

- [x] 1.1 Fix Windows cross-build for `connectivity/tcp_reuse_windows.go` and verify `GOOS=windows GOARCH=amd64 go build ./cmd/miopunch`.
- [x] 1.2 Add minimal build metadata support so tagged CI builds can report `v0.1.0-rc.1` through existing version surfaces.
- [x] 1.3 Verify Linux and Windows build commands for `cmd/miopunch`, `cmd/miopunch-lab`, `tools/miopunch-poc-e2e`, and `cmd/miopunch-desktop`.

## 2. Artifact Production

- [x] 2.1 Add or update build packaging logic for Linux amd64 CLI/lab bundles and Windows amd64 CLI/lab bundles.
- [x] 2.2 Update Linux `.deb` packaging so CI can set the package version from the release tag and build both WebKitGTK variants.
- [x] 2.3 Add Windows NSIS installer build steps that prepare `miopunch.exe`, `miopunch-desktop.exe`, and `miopunch-setup.exe`.
- [x] 2.4 Generate `checksums.txt` and `release-manifest.json` from the final release asset directory.
- [x] 2.5 Fix Wails desktop release builds to use production tags and Windows WebView2 embed mode.
- [x] 2.6 Register the Windows uninstaller in Apps & Features and add a Start menu uninstall shortcut.

## 3. GitHub Actions Workflows

- [x] 3.1 Add `go-checks.yml` for host gates and cross-build sanity.
- [x] 3.2 Add `build-artifacts.yml` for pure artifact builds and Actions artifact uploads.
- [x] 3.3 Add `lab-core-gates.yml` for `selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, and `xtcp-fulltest`.
- [x] 3.4 Add `lab-scenarios.yml` for `mnt01-fulltest`, `mnt02-selftest`, and `mnt03-fulltest`.
- [x] 3.5 Add `release.yml` for `v*` tag orchestration, prerelease publishing, checksums, manifest upload, asset attestations, and release-blocking host/build/core lab gates.
- [x] 3.6 Keep `lab-scenarios.yml` runnable as a standalone manual workflow without making it a release dependency.

## 4. CI Diagnostics and Permissions

- [x] 4.1 Configure hosted Ubuntu lab dependencies, Debian cloud image caching, and artifact upload for `lab/_artifacts/` plus QEMU logs.
- [x] 4.2 Scope workflow permissions so only publish/attestation jobs get write permissions.
- [x] 4.3 Ensure failed build or lab jobs leave enough uploaded evidence to diagnose the failing stage.

## 5. Validation

- [x] 5.1 Run `openspec validate --all --strict --no-interactive`.
- [x] 5.2 Run `export PATH=/usr/local/go/bin:$PATH && go test ./...`.
- [x] 5.3 Run `export PATH=/usr/local/go/bin:$PATH && go vet ./...`.
- [x] 5.4 Run `bash scripts/check_no_xtcp_imports.sh`.
- [x] 5.5 Run the release build sanity commands locally or through `build-artifacts.yml`.
- [x] 5.6 Verify the Windows Wails desktop production build no longer embeds the missing build tags fallback app.
- [ ] 5.7 Run the Windows installer smoke on a local Windows machine: install as Administrator, verify service/GUI startup, verify LocalAPI connection, verify Apps & Features uninstall entry, and uninstall.
- [x] 5.8 Run release-blocking core lab gates: `selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, and `xtcp-fulltest`.
- [ ] 5.9 Run scenario gates locally before tagging: `mnt01-fulltest`, `mnt02-selftest`, and `mnt03-fulltest`.
- [ ] 5.10 Push annotated tag `v0.1.0-rc.1` only after validation is green, then verify the GitHub Release assets and checksums.
- [x] 5.11 Run the Linux `.deb` desktop smoke locally: build/install the package, verify GUI startup and LocalAPI connection, and fix the Invite/Create stale task snapshot race found during smoke.

Note: 5.7, 5.8, 5.9, and 5.10 are intentionally left for the release/operator pass; this apply session implemented the automation and completed host/build/Linux desktop validation without publishing or archiving.
