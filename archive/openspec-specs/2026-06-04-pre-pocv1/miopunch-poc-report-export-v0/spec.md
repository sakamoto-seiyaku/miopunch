# miopunch-poc-report-export-v0 Specification

## Purpose
`miopunch-poc-report-export-v0` defines the minimal report/export behavior for POC tasks:

- Tasks produce a Markdown report at completion
- Reports can be fetched via LocalAPI
- CLI can export a report to a file for external sharing
- Optional redaction (`--redact`) supports safe external sharing

## Requirements

### Requirement: tasks have a Markdown report
For POC v0, tasks SHALL be able to expose a Markdown report generated at task completion, containing:

- Summary (task_id, kind, status, stage, reason_code, exit_code)
- Timeline
- Facts
- Suggestions

#### Scenario: Completed task exposes a report
- **WHEN** a POC task completes
- **THEN** the task exposes a Markdown report
- **AND** the report includes summary, timeline, facts, and suggestions sections

### Requirement: daemon persists recent reports
The daemon SHALL persist recent task reports under:

- `reports/<task_id>.md`

The daemon SHALL keep only the most recent N reports (default `reports_keep=20`) and delete older ones.

#### Scenario: Daemon rotates old task reports
- **WHEN** the daemon has more than `reports_keep` persisted task reports
- **THEN** it keeps the most recent reports
- **AND** it deletes older report files beyond the retention limit

### Requirement: CLI can export report to path
The CLI SHALL support exporting the report for the invoked task to a file path:

- `--report <path>`

#### Scenario: CLI writes report to requested path
- **WHEN** a user invokes a task command with `--report <path>`
- **THEN** the CLI writes the task report to that path
- **AND** the exported file is Markdown

### Requirement: redact avoids leaking secrets
When `--redact` is enabled, the CLI SHALL redact secret material (e.g., keys/secrets) from default text output and exported reports, using a stable and documented minimal redaction rule.

#### Scenario: Redacted export hides secret values
- **WHEN** a user exports a report with `--redact`
- **THEN** the CLI replaces secret material with redacted placeholders
- **AND** the exported report does not include raw key or secret values
