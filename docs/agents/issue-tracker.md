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

## Wayfinding operations

Current map: [miopunch 新基线：从已确认场景到可编写实施规格的决策地图](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/683de608-6db0-439e-b1ad-f6a83c28be59).

Current specification: [miopunch 新基线实施规格：身份、入网、连接、状态与迁移](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
It is a separate work item related to the map, labelled `ready-for-agent`,
and is the entry point for subsequent technical validation and implementation
planning. Its stated validation prerequisites are not completed capabilities.

The approved breakdown is published as 40 native children of this specification
(`MIOPUNCH-15` through `MIOPUNCH-54`), with native `blocked_by` relations.
These validation and implementation tickets belong to the specification, not
directly to the earlier decision map. Read their current states and dependencies
in Plane when choosing work. The initial entry points are
[native two-node integration](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/a451a208-eceb-40b4-817c-3893a9219a87)
and
[browser runtime and persistent identity](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/aee94cf2-4402-4c5d-8be0-a4edda96f61f).
Every ticket includes its first real check and an early failure check; passing
an unrelated validation ticket is not a prerequisite unless a blocking edge says so.

A decision map is a Plane work item labelled `wayfinder:map`. Decision
tickets use its UUID as their native `parent` and carry a
`wayfinder:grilling`, `wayfinder:research`, `wayfinder:prototype`, or
`wayfinder:task` label. Create type labels only as needed. The map is an
index; open work lives in its children, not a Markdown backlog.

Use `work-item list --json` to select children by `parent`. Query each
child's native dependencies with `work-item relations list <uuid>`.
The frontier consists of children outside the completed/cancelled state
groups, with no assignees and no blockers outside those closed groups.
Use `state list --json` to resolve state UUIDs and sort the frontier by
`sort_order`, then `sequence_id`, ascending. A triage label alone does not
establish eligibility. Apply `ready-for-human` to available HITL work and
`ready-for-agent` to available AFK work as the frontier changes.

Create children first, then add dependencies with
`work-item relations create <dependent-uuid> <blocker-uuid> --relation-type blocked_by`.
Dependencies are native UI edges, not body text. Relation removal is
currently unsupported on this instance; consult the plane-ops skill before
changing an existing edge.

Before working a ticket, assign it to the actual developer driving the map
using `work-item update <uuid> --raw-json '{"assignees":["<member-uuid>"]}'`.
Resolve the member through `member list`; do not invent an agent account.
Re-read the ticket and its dependencies before claiming it. Assignment is
the claim; never claim all children while charting.

On resolution, post the answer as a resolution comment, close the ticket,
and add its linked title and a one-line gist to the map's Decisions-so-far.
Link assets instead of copying them into the map. Existing repository
consensus is baseline evidence in Notes, not a newly resolved ticket.
Read back descriptions, labels, parent, state and dependency edges after
mutations. All commands retain the repo defaults flag documented above.
