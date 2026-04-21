## ADDED Requirements

### Requirement: Failures are explainable with stage, reason_code, facts, and suggestions
On any failure in POC commands and tasks, the system SHALL provide actionable output that includes:
- `stage`
- `reason_code`
- `facts`
- `suggestions`

#### Scenario: A task fails and returns actionable failure output
- **WHEN** a POC task fails
- **THEN** the result includes `stage` and `reason_code`
- **AND** `facts` contain at least one concrete observation
- **AND** `suggestions` contain at least one concrete user action

### Requirement: POC stage machine uses a fixed set of 8 stage identifiers
For tasks that report progress via the POC stage machine, the system SHALL use the following fixed stage identifiers (in order):
1. `ControlPlaneReady`
2. `SelfDiscovery`
3. `PeerContact`
4. `CandidateExchange`
5. `PunchAttempt`
6. `DataplaneHandshake`
7. `CapabilityHandshake`
8. `SessionAttach`

#### Scenario: A running task reports a stage from the fixed stage set
- **WHEN** a task is running and reports its current stage
- **THEN** the `stage` value is one of the 8 fixed stage identifiers

### Requirement: `--format json` outputs a one-line `miopunch.json.v0` envelope with stable top-level fields
When `--format json` is used, the system SHALL output a single-line JSON object with:
- `format` set to the fixed string `miopunch.json.v0`
- Stable top-level fields: `format`, `task_id`, `kind`, `status`, `stage`, `reason_code` (optional), `exit_code` (optional), `facts`, `suggestions`

`facts` and `suggestions` SHALL be arrays.
Each element of `facts` and `suggestions` SHALL be a JSON object with a required `message` string field.
Elements MAY include additional fields (e.g., `term_id`) for forward-compatible expansion.

#### Scenario: JSON output contains the stable envelope fields
- **WHEN** a command is run with `--format json`
- **THEN** the output is a single-line JSON object with `format: "miopunch.json.v0"`
- **AND** it contains the stable top-level fields

### Requirement: LocalAPI error responses include required fields and map exit_code to HTTP status
For LocalAPI requests, failures SHALL:
- Return an HTTP status that reflects failure
- Include an error response body with at least: `stage`, `reason_code`, `exit_code`, `message`, `facts`, `suggestions`, `request_id`

The server SHALL map `exit_code` to HTTP status using the following coarse mapping:
- `exit_code=2` → `400`
- `exit_code=3` → `503`
- `exit_code=4` → `403`
- `exit_code=5` → `504`
- `exit_code=6` → `409`
- `exit_code=7` → `404`
- `exit_code=1` → `500`

#### Scenario: LocalAPI failure returns both HTTP status and structured error body
- **WHEN** a LocalAPI request fails
- **THEN** the response has a non-2xx HTTP status
- **AND** the response body includes `stage`, `reason_code`, `exit_code`, `message`, `facts`, `suggestions`, and `request_id`

