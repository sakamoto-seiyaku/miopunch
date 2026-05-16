## Why

The repository is now mirrored to GitHub and needs a repeatable release path for the first public candidate tag. A single monolithic workflow would hide failures and make slow lab gates hard to reason about, so release automation must split build, test, lab, and publish responsibilities while still preventing a release unless all release-blocking gates pass.

## What Changes

- Add GitHub Actions workflows for separated responsibilities:
  - host Go checks and cross-build sanity;
  - pure artifact builds;
  - core lab gates;
  - scenario 1/2/3 lab gates;
  - tag-driven release orchestration.
- Publish `v0.1.0-rc.1` as a prerelease only after all release-blocking host, build, and core lab gates complete successfully.
- Keep scenario 1/2/3 gates available as a standalone workflow for manual diagnosis, but run them locally before tagging instead of making GitHub Release publishing depend on them.
- Release the current Door 1 Pro session-first artifacts:
  - Linux amd64 session bundle;
  - Windows amd64 session bundle;
  - checksums, a machine-readable release manifest, and provenance/attestation metadata.
- Keep tag creation outside Actions: the maintainer pushes an annotated `v*` tag to the current `origin`, and the local mirror chain synchronizes it to GitHub.
- Record accepted CI constraints: GitHub-hosted Ubuntu is used for lab workflows, even though nested QEMU/KVM behavior may be slower or less stable than a dedicated KVM runner.
- Keep `.deb` and NSIS scaffolding outside the default GitHub CI/release path; they remain deferred `D1a-privileged` packaging rather than the current Pro release contract.

## Capabilities

### New Capabilities

- `miopunch-release-automation-v0`: Defines GitHub Actions release automation, separated workflow responsibilities, release gating, artifact contracts, and first-release tag behavior.

### Modified Capabilities

- None.

## Impact

- Affected repository automation:
  - `.github/workflows/**` release, build, and gate workflows.
  - Current release helper scripts (`scripts/release/build_bundles.sh` and `scripts/release/generate_manifest.sh`) for deterministic session bundle outputs.
- Affected product code:
  - Minimal Windows build fix if required by cross-build sanity.
  - Minimal build-version metadata hook if required for tagged release binaries to report the release version.
- Affected validation:
  - Host checks remain `go test ./...`, `go vet ./...`, and `bash scripts/check_no_xtcp_imports.sh`, plus cross-build sanity for the current session bundle targets.
  - Release gate includes host checks, session bundle artifact builds, and core lab selftests before publishing.
  - Scenario gates remain required operator validation before tagging, but are not GitHub Release workflow dependencies.
- Out of scope:
  - Creating/pushing the release tag from CI.
  - Docker image publishing, Homebrew/RPM/macOS packages, signing certificates, and store distribution.
  - Default `.deb` or NSIS packaging in the GitHub release path.
