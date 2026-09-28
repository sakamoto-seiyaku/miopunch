## Why

The desktop GUI can start shell-related tasks, but the demo path is still a thin peer-detail action rather than a repeatable operator loop that demonstrates target discovery, attach, terminal status, disconnect, and retry. Tightening this loop at the desktop UI layer gives the project a stable shell demo without changing the existing `sh_ls` or `sh_attach` transport contracts.

## What Changes

- Make the desktop shell flow a repeatable demo loop: select peer, list targets/sessions, attach, observe terminal status, disconnect, and retry.
- Keep existing LocalAPI WebSocket `sh_attach` semantics and `miopunch.sh.v0` unchanged.
- Improve desktop UI state for shell task progress, terminal bridge failures, disconnect/retry, and selected target/session values.
- Add browser-level coverage using the fake bridge/runtime so the committed static UI validates the demo loop.
- Do not add a live lab gate in this change; existing lab shell coverage remains responsible for live transport validation.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `miopunch-desktop-gui-v0`: define the desktop shell demo loop and browser-test acceptance path.

## Impact

- Affected code: `cmd/miopunch-desktop/frontend/dist` and Playwright support/tests.
- Public behavior: desktop users get a clearer shell workflow and recoverable demo loop around existing shell tasks.
- APIs: no LocalAPI, shell protocol, or task kind changes are planned.
- Validation: Playwright desktop tests plus focused checks for touched frontend assets; full `$dev` gates apply before code-affecting implementation enters mainline.
