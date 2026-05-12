## Context

The current desktop shell path can reach a real Linux desktop ->
Windows-controlled peer attach attempt using existing `sh_ls`, `sh_attach`,
LocalAPI WebSocket attach, and remote shell target execution. In the failing
case under investigation, the chain progresses through candidate exchange,
punching, hello, shell attach request, and local WebSocket attach before the
operator finally sees `Disconnected: websocket closed (1006)`.

The problem is not that the shell entry point is missing. The problem is that
late attach failures lose detail while crossing several boundaries:

- remote shell target / tmux / PTY backend
- remote acceptor stream handling
- local task stream bridge
- LocalAPI WebSocket close
- desktop terminal bridge and UI status text

By the time the operator sees the failure, the original cause may have already
collapsed to `EOF` and then to WebSocket `1006`.

## Goals / Non-Goals

**Goals:**
- Preserve the best available shell attach failure cause across the existing
  desktop shell chain.
- Ensure post-attach shell failures still resolve to actionable
  `stage`/`reason_code`/`facts` output.
- Make the desktop shell UI show a concise failure summary for abnormal shell
  closure while keeping same-target retry available.
- Add intent-based logs (`debug` / `info` / `warn` / `error`) around the attach
  path so reruns can pinpoint the failing layer quickly.

**Non-Goals:**
- Do not redesign `miopunch.sh.v0`, add a new task kind, or replace the current
  `sh_attach` WebSocket flow.
- Do not solve the separate MQTT broker barrier timeout issue in this change.
- Do not redesign shell target discovery, tmux locking semantics, or desktop
  peer selection UX beyond what diagnostics require.
- Do not promise a functional fix for every shell target failure in the same
  change; the first objective is to make the real cause visible and attributable.

## Decisions

1. **Treat late shell attach failure as a diagnosable task outcome, not only a transport close.**
   The implementation should preserve structured shell failure data on the task
   path even when the interactive WebSocket or stream closes first. The desktop
   client can then use the final task result or close context to present a short
   diagnostic summary instead of a raw `1006`.

   Alternative considered: only improve frontend status text. Rejected because
   the frontend currently does not own the missing failure cause; it can only
   display what the lower layers preserve.

2. **Use the existing task identity as the correlation key across layers.**
   Logs and propagated facts should key off the existing `task_id` together with
   `peer`, `target`, and `session`, rather than inventing a new correlation
   protocol. This keeps the change small and makes desktop logs, daemon logs,
   task reports, and bridge events line up on one shared identifier.

   Alternative considered: add a new per-stream correlation ID. Rejected for the
   first pass because it expands the change surface without solving the primary
   attribution gap.

3. **Add layer-oriented shell diagnostics instead of overloading raw transport errors.**
   When a late attach failure happens, the preserved diagnostics should identify
   the shell layer that failed, such as `desktop_bridge`, `localapi_ws`,
   `task_bridge`, `acceptor`, `shelltarget`, `tmux`, `pty`, or `ssh`. Raw
   `EOF`, close code, or backend stderr can remain as supporting facts, but not
   as the only operator-facing explanation when richer diagnosis exists.

   Alternative considered: preserve only raw close errors. Rejected because that
   repeats the current ambiguity and forces operators to infer the failing hop.

4. **Standardize log level intent on the attach path.**
   - `debug`: frame-level or bridge lifecycle detail useful during focused
     debugging
   - `info`: successful attach milestones and orderly disconnects
   - `warn`: abnormal but bounded shell closure where the process remains
     healthy
   - `error`: unexpected failures that abort attach setup or lose required shell
     context

   Alternative considered: keep current ad hoc logging and only add more lines.
   Rejected because more volume without level intent will not improve rerun
   diagnosis.

5. **Keep protocol scope fixed.**
   The change should not alter `sh_attach` task creation semantics or
   `miopunch.sh.v0` frame format. Diagnostics should be carried through existing
   task state/report pathways and desktop bridge behavior.

   Alternative considered: add new WebSocket control frames for diagnostics.
   Rejected because it would couple the fix to protocol expansion instead of
   first using the contracts already present in task output.

## Risks / Trade-offs

- **[Risk] Richer logging could become noisy on healthy shells.**
  -> Mitigation: reserve high-volume detail for `debug`; keep `info`/`warn`/`error`
  focused on lifecycle milestones and attributable failures.

- **[Risk] Some late failures may still end without a fully classified root cause.**
  -> Mitigation: require layer attribution and final context when exact backend
  cause is unavailable; preserve raw close evidence as supplemental facts.

- **[Risk] Desktop UI could race the final task update after a WebSocket close.**
  -> Mitigation: design the desktop shell close path to query or subscribe for
  the final task outcome before falling back to generic disconnect text.

- **[Risk] Spec scope could accidentally absorb the broker timeout issue.**
  -> Mitigation: keep proposal, design, and tasks explicitly limited to the late
  shell attach path after discovery and candidate exchange already succeed.

## Migration Plan

No protocol or packaging migration is planned.

Implementation should roll out in this order:
1. Preserve/emit structured diagnostics in the remote shell attach path.
2. Propagate those diagnostics through task state/report and LocalAPI-facing
   bridges.
3. Update the desktop shell bridge/UI to render the concise summary and retry
   state.
4. Add focused validation for the new diagnostic behavior.

Rollback is low risk because the change is additive around diagnostics and UI
presentation on existing flows.

## Open Questions

- Do we need one or more new stable `reason_code` values for late post-attach
  shell termination, or can the existing shell codes cover the initial pass when
  paired with stronger `facts`?
- Should the desktop UI fetch final task state immediately on abnormal close, or
  should the desktop bridge own that enrichment and return one already-resolved
  close summary?
- How much backend stderr/stdout detail from shell targets is safe to surface in
  operator-visible facts without creating noisy or unstable UX text?
