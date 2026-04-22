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

### Requirement: daemon persists recent reports
The daemon SHALL persist recent task reports under:

- `reports/<task_id>.md`

The daemon SHALL keep only the most recent N reports (default `reports_keep=20`) and delete older ones.

### Requirement: CLI can export report to path
The CLI SHALL support exporting the report for the invoked task to a file path:

- `--report <path>`

### Requirement: redact avoids leaking secrets
When `--redact` is enabled, the CLI SHALL redact secret material (e.g., keys/secrets) from default text output and exported reports, using a stable and documented minimal redaction rule.

