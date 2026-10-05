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

### Rust builds and disk space

Read `docs/agents/build-cache.md` before compiling. Use the shared repository
`target/` through `scripts/cargo_cache.py`; keep per-run receipts separate.
Do not copy a Cargo target tree or create a new target for each check. Keep
the committed profile settings and stable Android toolchain paths. Preserve
raw evidence and pinned SDK artifacts when cleaning regenerable caches.

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
