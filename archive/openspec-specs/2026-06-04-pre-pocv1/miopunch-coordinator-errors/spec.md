# miopunch-coordinator-errors Specification

## Purpose
TBD - created by archiving change remove-xtcp-from-coordinator-errors. Update Purpose after archive.
## Requirements
### Requirement: Coordinator error messages do not expose xtcp naming
The system SHALL NOT include the substring `xtcp` in user-visible error messages returned by the coordinator (`internal/coordinator`) during punching precheck, auth, and session setup. Errors SHOULD use `miopunch` naming or neutral terms such as `proxy` / `peer`.

#### Scenario: Visitor precheck fails because proxy is missing
- **WHEN** a visitor performs precheck for a `proxy_name` that is not registered by any client
- **THEN** the coordinator returns an error message that does not contain `xtcp`

#### Scenario: Visitor is not allowed for the proxy
- **WHEN** a visitor requests a proxy that disallows the visitor user
- **THEN** the coordinator returns an error message that does not contain `xtcp`

