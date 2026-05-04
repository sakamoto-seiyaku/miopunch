## Context

`miopunch-desktop` currently embeds committed static assets from `cmd/miopunch-desktop/frontend/dist`. A scoped Playwright package already exists for the invite Create regression, but its fixture is local to one spec and only covers Access -> Invite -> Create.

The desktop UI is intentionally reviewed through browser-rendered static assets. The test strategy must therefore validate DOM behavior and bridge interactions without requiring a running daemon, a real Wails shell, or a frontend build step.

## Goals / Non-Goals

**Goals:**

- Cover the primary desktop interaction model across Network, Access, Admin, and Settings.
- Verify role-gated controls, disabled states, bridge call arguments, runtime event rendering, and recoverable error states.
- Make fake Wails bridge setup reusable so future UI regressions can be tested cheaply.
- Record product/UI defects found by the expanded suite in `findings.md` before fixing them.

**Non-Goals:**

- Do not automate a real Wails desktop window in this change.
- Do not add screenshot baselines or visual regression approval workflows.
- Do not introduce a frontend source build pipeline.
- Do not change daemon, LocalAPI, or Wails bridge contracts.
- Do not fix test-discovered product UI defects unless the fix is explicitly requested or the defect is in the test harness itself.

## Decisions

- Use Playwright against `frontend/dist` with an injected fake `window.go.main.App`. This exercises real DOM clicks and rendering while staying deterministic in CI.
- Move shared bridge fixtures and app helpers into test support modules. Specs should describe user intent, not rebuild topology objects or bridge methods inline.
- Fail tests on browser `pageerror` and unexpected console errors. Silent JavaScript errors are UI regressions even when the visible assertion still passes.
- Separate behavioral assertions from defect logging. If a newly added test reveals a product defect, record it in `findings.md` and either mark the test as documenting the current limitation or narrow the test to stable expected behavior until the defect is accepted for fixing.
- Keep CI as `npm test` under `cmd/miopunch-desktop/frontend`; the existing workflow job remains the single desktop UI gate.

## Risks / Trade-offs

- Fake bridge tests do not prove Wails runtime integration -> existing Go bridge tests and packaging checks remain responsible for that layer.
- Broad UI tests can become brittle if they assert presentation details -> prefer role/text/state/call assertions over CSS layout internals.
- Recording discovered issues without fixing them can leave known defects in place -> `findings.md` makes those defects explicit and reviewable before product changes are made.
- Tests target committed dist assets -> contributors must update tests when directly editing dist behavior.
