# miopunch-release-automation-v0 Specification

## Purpose
Define the current GitHub Actions release automation contract for the POC v1 session/demo release path.

## Requirements
### Requirement: Release automation uses separated workflows
The system SHALL keep host checks, artifact builds, optional lab/debug gates, and release publishing in separate GitHub Actions workflows.

#### Scenario: Workflows are independently runnable
- **WHEN** a maintainer opens the GitHub Actions workflow list
- **THEN** host checks, artifact builds, optional lab/debug gates, and release publishing are exposed as distinct workflows
- **AND** each non-release workflow can be run independently for diagnosis

#### Scenario: Release workflow orchestrates required gates
- **WHEN** a supported release tag triggers the release workflow
- **THEN** the release workflow waits for host checks and artifact builds before publishing
- **AND** no GitHub Release is created if any required gate fails
- **AND** the release workflow does not depend on historical VM lab gates

### Requirement: Host checks validate Go quality before release
The system SHALL run the repository host validation gates before a release can publish.

#### Scenario: Host checks pass
- **WHEN** the host checks workflow runs for a release candidate commit
- **THEN** it runs `go test ./...`
- **AND** it runs `go vet ./...`
- **AND** it runs `bash scripts/check_no_xtcp_imports.sh`
- **AND** it performs release cross-build sanity for the current Linux, Windows, and Android POC v1 assets when those build paths are available

#### Scenario: Host checks fail
- **WHEN** any host validation command or cross-build sanity command fails
- **THEN** the host checks workflow fails
- **AND** the release workflow cannot publish a release for that commit

### Requirement: Build workflow produces release candidate assets
The system SHALL provide a pure build workflow that compiles and packages the current POC v1 release assets without publishing a GitHub Release.

#### Scenario: Build workflow creates CI artifacts
- **WHEN** the artifact build workflow runs
- **THEN** it builds `miopunch_<version>_linux_amd64_session.tar.gz`
- **AND** it builds `miopunch_<version>_windows_amd64_session.zip`
- **AND** it builds `miopunch_<version>_android_arm64_control-lite-debug.apk` when the Android build path is available
- **AND** desktop release binaries use Wails production build tags
- **AND** it uploads those outputs as GitHub Actions artifacts

#### Scenario: Windows desktop asset uses Wails production mode
- **WHEN** the Windows session bundle workflow builds `miopunch-desktop.exe`
- **THEN** the binary is built with `desktop,production,wv2runtime.embed` tags
- **AND** the binary is linked as a Windows GUI executable
- **AND** the binary does not include Wails' missing build tags fallback app

#### Scenario: Build workflow records integrity metadata
- **WHEN** release candidate assets are produced
- **THEN** the workflow writes `checksums.txt`
- **AND** it writes a machine-readable `release-manifest.json`
- **AND** the checksums cover every published asset

#### Scenario: Deferred privileged packaging stays out of the default release path
- **WHEN** the default artifact build workflow runs
- **THEN** it does not build Linux `.deb` packages
- **AND** it does not build a Windows NSIS installer
- **AND** the default GitHub artifact contract matches the local `build_bundles.sh` session bundle flow

### Requirement: Lab gates are optional debug validation for current POC v1 releases
The current POC v1 release path SHALL NOT gate release publishing on legacy/core VM lab gates.

Historical lab commands MAY remain available for manual diagnosis, but they SHALL NOT be described as required current POC v1 release gates until a future POC v1 lab capability redefines them.

#### Scenario: Current release gate skips legacy VM lab commands
- **WHEN** a current POC v1 release candidate is validated
- **THEN** required validation does not run `labctl selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`, `mnt01-fulltest`, `mnt02-selftest`, or `mnt03-fulltest`
- **AND** maintainers may still run those commands manually for historical/debug investigation

#### Scenario: Lab artifacts are retained for failure diagnosis
- **WHEN** any lab gate completes or fails
- **THEN** the workflow uploads `lab/_artifacts/`
- **AND** it uploads QEMU host logs needed to diagnose VM startup or guest failures when present

### Requirement: Release publishing is tag-driven and guarded
The system SHALL publish GitHub Releases only from supported `v*` tags and SHALL NOT create or move release tags from CI.

#### Scenario: Maintainer pushes release tag
- **WHEN** a maintainer pushes annotated tag `v0.1.0-rc.1` to the configured `origin`
- **THEN** the GitHub mirror receives the tag
- **AND** the release workflow treats the tag as the release source
- **AND** the workflow publishes a prerelease with `latest=false` after host checks and artifact builds pass

#### Scenario: Unsupported tag does not publish
- **WHEN** a tag does not match the supported release tag policy
- **THEN** the release workflow does not publish a GitHub Release

### Requirement: Release assets include provenance and least-privilege permissions
The system SHALL publish release assets with provenance metadata and SHALL use least-privilege GitHub Actions permissions.

#### Scenario: Release assets include attestations
- **WHEN** the release workflow publishes assets
- **THEN** it attaches or generates provenance/attestation metadata for published assets
- **AND** maintainers can verify the assets against the release commit

#### Scenario: Workflow permissions are scoped
- **WHEN** non-publish workflows run
- **THEN** they use read-only repository permissions unless a stronger permission is required for artifact handling
- **AND** only the publish job uses permissions needed to create the GitHub Release and attest assets

### Requirement: Lab workflows document historical/debug constraints
The system SHALL document that legacy VM lab gates are historical/debug validation for current POC v1 until a future POC v1 lab capability redefines them.

#### Scenario: Hosted runner lacks fast KVM for manual lab debugging
- **WHEN** a maintainer runs historical VM lab commands on a hosted runner without usable `/dev/kvm`
- **THEN** lab execution may fall back to QEMU TCG
- **AND** timeout or gate failure does not block current POC v1 release publishing
- **AND** uploaded artifacts identify the failing stage as far as the lab can report it
