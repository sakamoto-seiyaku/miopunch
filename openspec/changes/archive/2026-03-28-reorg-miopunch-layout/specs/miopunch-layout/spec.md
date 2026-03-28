## ADDED Requirements

### Requirement: Remove Top-Level xtcp Namespace
The system SHALL eliminate the top-level `xtcp/` Go package namespace from this repository as part of `P3` naming and layout convergence.
New code SHALL NOT introduce new public-facing packages or CLI terminology under `xtcp`.

#### Scenario: No xtcp imports remain after migration
- **WHEN** a developer searches the repository for Go imports under `github.com/miopunch/miopunch/xtcp`
- **THEN** the search returns no matches

### Requirement: Miopunch-Oriented Package Layout
The system SHALL reorganize code into stable top-level domain packages and move glue and implementation details into `internal/`.
At minimum, the repository SHALL provide top-level packages aligned with `P3` (`connectivity`, `event`, `nat`, `stun`) and SHALL use `internal/` for coordinator/peer/wire helpers.

#### Scenario: Top-level domain packages exist
- **WHEN** a developer inspects the repository structure
- **THEN** the repository contains `connectivity/`, `event/`, `nat/`, and `stun/`
- **AND** coordinator/peer/wire helpers live under `internal/`

### Requirement: Single Binary Entry Point Uses miopunch Naming
The system SHALL keep `cmd/miopunch` as the single binary entry point.
The CLI usage/help text SHALL use `miopunch` naming and SHALL NOT present itself as `xtcp`.

#### Scenario: Help text does not expose xtcp naming
- **WHEN** a developer prints CLI help (e.g. `miopunch help`)
- **THEN** the help output uses `miopunch` terminology
- **AND** the output does not contain `xtcp`

### Requirement: Preserve Go Build and Testability After Layout Migration
The system SHALL remain buildable and testable after the directory reorganization.

#### Scenario: Go tests pass after migration
- **WHEN** the developer runs `go test ./...`
- **THEN** the test run succeeds

### Requirement: Update Active Docs to New Change Paths
The system SHALL update actively maintained docs to reference the current OpenSpec change locations after archiving and to reflect `P3` naming.
Historical reports and historical decisions are out of scope for mechanical rewrites.

#### Scenario: Roadmap references current change locations
- **WHEN** a developer reads `docs/roadmap.md`
- **THEN** references to OpenSpec changes use the correct current paths under `openspec/changes/`
- **AND** `P3` language uses `miopunch` naming

