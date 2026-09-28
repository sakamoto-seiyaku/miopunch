# Issue Tracker: Plane

Issues, PRDs, decision tickets, and implementation tasks for this repository
live in Plane project `MIOPUNCH` (workspace `work`). Use the global
`plane-ops` skill and its REST CLI. Do not use Plane MCP.

The global CLI defaults point to another project. For every miopunch tracker
operation, pass `--defaults docs/agents/plane-defaults.json` before the
subcommand (from the repository root). This file contains only non-sensitive
settings; the API key stays in the OS keyring. Do not copy the other project's
state or label UUIDs into this file.

For example:

```bash
/home/js/.agents/skills/plane-ops/scripts/plane-ops.sh \
  --defaults docs/agents/plane-defaults.json work-item list
```

Repository docs may retain evidence, acceptance criteria, and links to Plane
work items. Do not maintain an open-work ledger or use `.scratch/` as a second
source of truth. OpenSpec proposals and task lists in the legacy archive are
historical context, not current tasks.
