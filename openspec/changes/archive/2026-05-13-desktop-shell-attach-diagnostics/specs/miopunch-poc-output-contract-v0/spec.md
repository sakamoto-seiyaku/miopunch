## ADDED Requirements

### Requirement: Interactive shell failures remain actionable after attach
The final task output SHALL remain actionable when an interactive `sh_attach`
task ends abnormally after attach setup has started, even if the operator first
experiences the failure as a closed interactive stream.

The final output SHALL still include:
- `stage`
- `reason_code`
- `facts`
- `suggestions`

For late shell attach failures, the output SHALL preserve enough facts for an
operator to identify the selected peer, target, session, and last known failing
shell layer.

#### Scenario: Post-attach shell failure still exports actionable output
- **WHEN** an interactive `sh_attach` session closes unexpectedly after attach
  setup
- **THEN** the final task output still includes `stage`, `reason_code`, `facts`,
  and `suggestions`
- **AND** the facts identify the selected peer, target, session, and last known
  failing shell layer

#### Scenario: Generic stream close still yields actionable suggestions
- **WHEN** an interactive `sh_attach` session closes with only generic stream
  evidence available
- **THEN** the final task output still contains at least one concrete fact and
  one concrete suggestion
- **AND** the operator does not need daemon log access to know which shell path
  failed
