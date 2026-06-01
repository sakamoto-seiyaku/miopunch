## 1. Runtime Authority

- [x] 1.1 Consume `miopunch-poc-v1-headless-runtime` as the only extracted-v1 runtime authority for stage, gate, and failure surfaces.
- [x] 1.2 Project runtime `summary` and structured `evidence` into the default GUI output layers without re-defining the underlying contracts.
- [x] 1.3 Keep GUI-specific view state separate from runtime authority and legacy desktop internals.

## 2. Desktop Integration

- [x] 2.1 Rewire the desktop runtime to consume the shared daemon `localapi` RPC plus dedicated event/shell streams instead of directly composing legacy task internals or using a CLI bridge.
- [x] 2.2 Reuse existing desktop/localapi/desktopbridge code only as shell/plumbing.
- [x] 2.3 Keep GUI rendering as the default presentation layer for peer list, punch evidence, session state, and shell-stage summaries.

## 3. Acceptance

- [x] 3.1 Add tests for four-tab GUI navigation, summary/evidence presentation, runtime-event consumption, invite visibility/copy, and shell attach behavior after the headless-runtime gate succeeds.
- [x] 3.2 Add Linux desktop smoke covering the control-console path through the v1 runtime API, including `Network` bootstrap, `Admin` invite flow, `Shell` ping gate, and diagnostics export.
- [x] 3.3 Add Windows desktop smoke for GUI startup, daemon connection, and runtime contract consumption; do not make Windows/Linux real-machine interoperability a 07 blocker.
