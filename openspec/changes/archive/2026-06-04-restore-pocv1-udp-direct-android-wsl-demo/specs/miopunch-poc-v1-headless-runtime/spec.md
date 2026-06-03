## ADDED Requirements

### Requirement: Current v1 runtime exposes selected UDP path evidence
The current v1 runtime SHALL expose the selected UDP path in structured success and failure evidence for `ping`, `sh ls`, and `sh` flows that establish a peer session.

The exposed path value SHALL distinguish at least:

- `direct_ipv4`
- `punching_ipv4`

The CLI JSON output and report output SHALL preserve this evidence so an operator can tell whether an Android/WSL demo succeeded by LAN-direct UDP or by UDP punching.

#### Scenario: Ping output reports direct UDP selection
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP direct reachability
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=direct_ipv4`

#### Scenario: Punching output reports punching selection
- **WHEN** `miopunch ping <peer>` establishes a new peer session through UDP punching fallback
- **THEN** the command succeeds
- **AND** its structured facts or report data include `selected_path=punching_ipv4`

#### Scenario: Failure evidence remains stage-locatable
- **WHEN** current v1 peer session establishment fails after trying UDP direct reachability and UDP punching
- **THEN** the failure output includes `stage`, `reason_code`, `facts`, and `suggestions`
- **AND** the facts identify candidate-pair evidence well enough to distinguish direct timeout from punching timeout
