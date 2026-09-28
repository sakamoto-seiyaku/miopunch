@/home/js/.codex/RTK.md

# miopunch agent rules

The repository root is a new design baseline. The former Go/Android code,
OpenSpec material, and project-local development skills are under
`archive/legacy-go-openspec-2026-09-28/` without a current implementation
role. Do not apply the old Go/OpenSpec workflow or its test gates to new work
by default. Whether old tests and scripts should be reused, adapted, or
replaced is undecided.

Do not turn research suggestions into product decisions. Read the current
design entry points before planning implementation, and record new decisions
explicitly. Use Plane for tasks; never use Plane MCP.

## Agent skills

### Issue tracker

Issues, PRDs, and decision tickets live in Plane's `MIOPUNCH` project. Use the
global `plane-ops` skill with this repo's defaults, not Plane MCP. See
`docs/agents/issue-tracker.md`.

### Triage labels

Use the five default triage roles in Plane. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repo. Read `CONTEXT.md`, the current design notes,
and relevant ADRs before architectural work. Archived Go/OpenSpec material is
historical only. See `docs/agents/domain.md`.
