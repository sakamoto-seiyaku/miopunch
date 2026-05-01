## ADDED Requirements

### Requirement: Release automation uses separated workflows
The system SHALL keep host checks, artifact builds, core lab gates, scenario lab gates, and release publishing in separate GitHub Actions workflows.

#### Scenario: Workflows are independently runnable
- **WHEN** a maintainer opens the GitHub Actions workflow list
- **THEN** host checks, artifact builds, core lab gates, scenario lab gates, and release publishing are exposed as distinct workflows
- **AND** each non-release workflow can be run independently for diagnosis

#### Scenario: Release workflow orchestrates required gates
- **WHEN** a supported release tag triggers the release workflow
- **THEN** the release workflow waits for host checks, artifact builds, core lab gates, and scenario lab gates before publishing
- **AND** no GitHub Release is created if any required gate fails

### Requirement: Host checks validate Go quality before release
The system SHALL run the repository host validation gates before a release can publish.

#### Scenario: Host checks pass
- **WHEN** the host checks workflow runs for a release candidate commit
- **THEN** it runs `go test ./...`
- **AND** it runs `go vet ./...`
- **AND** it runs `bash scripts/check_no_xtcp_imports.sh`
- **AND** it performs release cross-build sanity for supported release targets

#### Scenario: Host checks fail
- **WHEN** any host validation command or cross-build sanity command fails
- **THEN** the host checks workflow fails
- **AND** the release workflow cannot publish a release for that commit

### Requirement: Build workflow produces release candidate assets
The system SHALL provide a pure build workflow that compiles and packages release candidate assets without publishing a GitHub Release.

#### Scenario: Build workflow creates CI artifacts
- **WHEN** the artifact build workflow runs
- **THEN** it builds Linux amd64 CLI/lab binary bundles
- **AND** it builds Windows amd64 CLI/lab binary bundles
- **AND** it builds Linux WebKitGTK 4.0 and 4.1 `.deb` packages
- **AND** it builds a Windows NSIS installer
- **AND** it uploads those outputs as GitHub Actions artifacts

#### Scenario: Build workflow records integrity metadata
- **WHEN** release candidate assets are produced
- **THEN** the workflow writes `checksums.txt`
- **AND** it writes a machine-readable `release-manifest.json`
- **AND** the checksums cover every published asset

### Requirement: Lab gates cover core and scenario validation
The system SHALL gate release publishing on both legacy/core lab tests and scenario 1/2/3 lab tests.

#### Scenario: Core lab gates run before release
- **WHEN** the core lab gate workflow runs for a release candidate commit
- **THEN** it runs `./lab/host/labctl selftest`
- **AND** it runs `./lab/host/labctl xtcp-selftest`
- **AND** it runs `./lab/host/labctl xtcp-connectivity-selftest`
- **AND** it runs `./lab/host/labctl xtcp-fulltest`

#### Scenario: Scenario lab gates run before release
- **WHEN** the scenario lab gate workflow runs for a release candidate commit
- **THEN** it runs scenario 1 through `./lab/host/labctl mnt01-fulltest`
- **AND** it runs scenario 2 through `./lab/host/labctl mnt02-selftest`
- **AND** it runs scenario 3 through `./lab/host/labctl mnt03-fulltest`

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
- **AND** the workflow publishes a prerelease with `latest=false` after all required gates pass

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

### Requirement: Lab workflows document hosted-runner constraints
The system SHALL document that release lab gates run on GitHub-hosted Ubuntu runners for v0, despite nested virtualization limits.

#### Scenario: Hosted runner lacks fast KVM
- **WHEN** the hosted runner lacks usable `/dev/kvm`
- **THEN** lab execution may fall back to QEMU TCG
- **AND** the workflow still treats a timeout or gate failure as release-blocking
- **AND** uploaded artifacts identify the failing stage as far as the lab can report it
