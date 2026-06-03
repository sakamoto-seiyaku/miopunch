## MODIFIED Requirements

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

### Requirement: Lab gates are optional debug validation for current POC v1 releases
The current POC v1 release path SHALL NOT gate release publishing on legacy/core VM lab gates.

Historical lab commands MAY remain available for manual diagnosis, but they SHALL NOT be described as required current POC v1 release gates until a future POC v1 lab capability redefines them.

#### Scenario: Current release gate skips legacy VM lab commands
- **WHEN** a current POC v1 release candidate is validated
- **THEN** required validation does not run `labctl selftest`, `xtcp-selftest`, `xtcp-connectivity-selftest`, `xtcp-fulltest`, `mnt01-fulltest`, `mnt02-selftest`, or `mnt03-fulltest`
- **AND** maintainers may still run those commands manually for historical/debug investigation

### Requirement: POC v1 release artifacts include desktop and Android demo assets
The system SHALL build and publish the current POC v1 session/demo assets for release.

The release asset set SHALL include Linux and Windows session bundles plus the Android control-lite debug APK when the Android build path is available.

#### Scenario: Release artifacts include Android control-lite
- **WHEN** release candidate assets are produced for the current POC v1 demo
- **THEN** the artifact build produces the Linux session bundle, Windows session bundle, and Android control-lite APK
- **AND** checksums and manifest metadata include every produced asset
