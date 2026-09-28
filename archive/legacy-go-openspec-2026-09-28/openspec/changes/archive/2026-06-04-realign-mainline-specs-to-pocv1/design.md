## Context

Current POC v1 has already moved to a demo/product path centered on `cmd/miopunch`, the shared daemon, LocalAPI, POC v1 runtime modules, desktop GUI, and Android control-lite. The active specs still include older experimental phases and mainline-network-test gates that were valid for earlier repository states but now point agents toward the wrong validation and implementation model.

Several completed changes are also still active rather than synced into main specs, so the current source of truth is split between `openspec/specs` and completed `openspec/changes/*` directories.

## Goals / Non-Goals

**Goals:**

- Make active `openspec/specs` represent the current POC v1 mainline.
- Preserve old specs as historical reference outside active specs.
- Sync completed POC v1 demo capabilities into main specs.
- Keep validation guidance aligned with the current host-check plus real-demo evidence gate.

**Non-Goals:**

- Do not delete historical specifications permanently.
- Do not rewrite Go implementation or test code.
- Do not resurrect VM lab gates as current requirements in this change.
- Do not redefine future TCP Door-2, MNT, or XTCP work beyond marking it historical/deferred.

## Decisions

1. Move stale capabilities to `archive/openspec-specs/2026-06-04-pre-pocv1/` instead of leaving them active with historical wording.
   - Rationale: active specs are consumed as current constraints; leaving stale specs active is the bug being fixed.
   - Alternative rejected: keep them in `openspec/specs` with historical notes, because that still lets validators and agents treat them as current.

2. Add one explicit `miopunch-poc-v1-current-mainline` capability.
   - Rationale: POC v1 needs a short active source that ties together runtime, CLI, GUI, Android, UDP-only pathing, and validation.
   - Alternative rejected: infer current mainline from many separate POC v1 specs, because that leaves the gate and historical boundary ambiguous.

3. Sync completed POC v1 changes before archiving them.
   - Rationale: Android control-lite, GUI console, UDP owner/session lifecycle, desktop shutdown, and direct Android/WSL behavior are already implemented and should not remain as pending proposals.

4. Treat VM lab gates as historical/debug until a future POC v1 lab change is defined.
   - Rationale: current POC v1 specs and validation evidence are not aligned to the old VM lab cases; running those gates now creates false blockers.

## Risks / Trade-offs

- Moving many specs at once can hide a still-current requirement. Mitigation: keep code-health, layout, release, desktop, POC v1, public reachability, punching decision, UDP owner/demux, and Windows/WSL smoke specs active.
- Release specs will no longer require legacy VM gates. Mitigation: the requirement will explicitly state host checks plus real-demo evidence and defer VM lab validation to a future POC v1 lab capability.
- Some docs still mention old phases as history. Mitigation: update the main OpenSpec project context, roadmap, and lab README now; leave deeper historical docs unchanged.
