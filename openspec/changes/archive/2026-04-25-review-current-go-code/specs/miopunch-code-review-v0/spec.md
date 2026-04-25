## ADDED Requirements

### Requirement: Review execution is apply-only
The change SHALL NOT start the actual Go code review during change creation. The actual review work MUST begin only when the change is applied.

#### Scenario: Creating the review change
- **WHEN** the review change is created
- **THEN** the change contains only OpenSpec planning artifacts and no review findings report

#### Scenario: Applying the review change
- **WHEN** the review change is applied
- **THEN** the reviewer runs the planned checks, reads the target code, and produces the findings report

### Requirement: Review output is findings-only
The apply phase SHALL produce a review findings report and MUST NOT modify Go source, tests, runtime scripts, public APIs, or behavior as part of the review.

#### Scenario: Review discovers issues
- **WHEN** the apply phase identifies code issues
- **THEN** it records them in `findings.md` without patching the affected code

#### Scenario: Follow-up fixes are needed
- **WHEN** a finding requires implementation work
- **THEN** the fix is deferred to a separate follow-up change

### Requirement: Findings are evidence-based
The findings report SHALL group issues by severity and each finding MUST include a concrete file line reference, rule category, impact statement, and enough context for a follow-up fix.

#### Scenario: Reviewer cannot prove an issue
- **WHEN** a suspected issue cannot be justified with a specific line reference and rule category
- **THEN** it is omitted from `findings.md`

#### Scenario: Reviewer records an issue
- **WHEN** a genuine issue is recorded
- **THEN** the finding includes severity, `file:line`, category, description, and impact

### Requirement: Review scope covers current Go codebase
The apply phase SHALL review the current repository Go codebase broadly, including Go packages, tests, `cmd/`, `tools/`, and key execution-affecting scripts relevant to Go behavior.

#### Scenario: Applying the review
- **WHEN** the reviewer starts the review
- **THEN** the review baseline commit is recorded before checks and manual review begin

#### Scenario: Reviewing concurrent code
- **WHEN** the reviewed code uses goroutines, channels, mutexes, WaitGroups, atomics, contexts, or shared state
- **THEN** the reviewer applies the Go concurrency review rules explicitly
