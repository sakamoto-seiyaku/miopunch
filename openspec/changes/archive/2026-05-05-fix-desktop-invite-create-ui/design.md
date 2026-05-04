## Context

`miopunch-desktop` embeds committed static assets from `cmd/miopunch-desktop/frontend/dist` and talks to the daemon through Wails-bound Go methods. Existing tests cover LocalAPI and desktop bridge helpers, but not the browser DOM flow where a user opens Access, selects Invite, and clicks Create.

The observed failure is a UI-level dead end: the Create button enters a busy state and the page remains at "Creating invite..." with no code and no visible recovery path. The daemon LocalAPI contract already supports `invite`, and CLI/localapi smoke tests cover that path.

## Goals / Non-Goals

**Goals:**

- Make the invite Create action always resolve to either a rendered task/code or a visible recoverable error.
- Preserve the existing LocalAPI task contract.
- Add a minimal browser smoke test for the desktop Access invite click flow.

**Non-Goals:**

- Do not introduce a frontend build step.
- Do not automate a real Wails desktop window in this change.
- Do not change daemon task execution semantics.

## Decisions

- Normalize empty task args in the desktop frontend before calling the Wails bridge. Passing an object for no-arg tasks avoids bridge ambiguity while preserving LocalAPI behavior.
- Add a UI-side timeout wrapper around bridge calls. Backend calls already have Go contexts, but the browser needs its own recovery if the Wails Promise does not settle.
- Use Playwright against `frontend/dist` with an injected fake `window.go.main.App`. This tests real DOM clicks and rendering while staying independent of a running daemon or Wails shell.
- Keep tests local to `cmd/miopunch-desktop/frontend` so the rest of the repo remains Go-first and the desktop static asset model stays unchanged.

## Risks / Trade-offs

- Playwright adds a Node test dependency. Mitigation: scope it to the desktop frontend folder and run only browser smoke tests there.
- Fake bridge tests do not prove Wails runtime behavior. Mitigation: they cover the missing DOM event/render path, while existing Go tests continue to cover bridge and LocalAPI behavior.
- UI timeout duration could be too short for slow machines. Mitigation: use a conservative timeout for task creation and only surface an error after the bridge fails to settle.
