## 1. Build Readiness

- [x] 1.1 Fix Windows cross-build for `connectivity/tcp_reuse_windows.go` and verify `GOOS=windows GOARCH=amd64 go build ./cmd/miopunch`.
- [x] 1.2 Add minimal build metadata support so tagged CI builds can report `v0.1.0-rc.1` through existing version surfaces.
- [x] 1.3 Verify Linux and Windows build commands for the current session bundle targets: `cmd/miopunch` and `cmd/miopunch-desktop`.

## 2. Artifact Production

- [x] 2.1 Keep the default release packaging logic on the current Linux amd64 and Windows amd64 session bundles.
- [x] 2.2 Generate `checksums.txt` and `release-manifest.json` from the final release asset directory.
- [x] 2.3 Fix Wails desktop release builds to use production tags and Windows WebView2 embed mode.
- [x] 2.4 Keep deferred `.deb` and NSIS packaging scaffolds out of the default GitHub artifact and release path.

## 3. GitHub Actions Workflows

- [x] 3.1 Add `go-checks.yml` for host gates and cross-build sanity.
- [x] 3.2 Add `build-artifacts.yml` for pure session-bundle artifact builds, manifest assembly, and Actions artifact uploads.
- [x] 3.3 Add `lab-core-gates.yml` for `selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, and `xtcp-fulltest`.
- [x] 3.4 Add `lab-scenarios.yml` for `mnt01-fulltest`, `mnt02-selftest`, and `mnt03-fulltest`.
- [x] 3.5 Add `release.yml` for `v*` tag orchestration, prerelease publishing, session bundle assets, checksums, manifest upload, asset attestations, and release-blocking host/build/core lab gates.
- [x] 3.6 Keep `lab-scenarios.yml` runnable as a standalone manual workflow without making it a release dependency.

## 4. CI Diagnostics and Permissions

- [x] 4.1 Configure hosted Ubuntu lab dependencies, Debian cloud image caching, and artifact upload for `lab/_artifacts/` plus QEMU logs.
- [x] 4.2 Scope workflow permissions so only publish/attestation jobs get write permissions.
- [x] 4.3 Ensure failed build or lab jobs leave enough uploaded evidence to diagnose the failing stage.

## 5. Validation

- [x] 5.1 Run `openspec validate --all --strict --no-interactive`.
- [x] 5.2 Run focused host validation for the current release path: `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`.
- [x] 5.3 Run `bash scripts/check_session_bundle_no_privileged.sh`.
- [x] 5.4 Run the release build sanity commands locally or through `build-artifacts.yml`.
- [x] 5.5 Verify the Windows Wails desktop production build no longer embeds the missing build tags fallback app.
- [x] 5.6 Run release-blocking core lab gates: `selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, and `xtcp-fulltest`.
- [ ] 5.7 Run the Windows session bundle smoke on a local Windows machine as a normal user: verify GUI startup, LocalAPI connection, and current session-first flow without an installer.
- [ ] 5.8 Run scenario gates locally before tagging: `mnt01-fulltest`, `mnt02-selftest`, and `mnt03-fulltest`.
- [ ] 5.9 Push annotated tag `v0.1.0-rc.1` only after validation is green, then verify the GitHub Release session bundles, manifest, and checksums.

Note: 5.7, 5.8, and 5.9 are intentionally left for the release/operator pass; this apply session keeps the automation aligned with the current session-first Pro contract without publishing or archiving.
