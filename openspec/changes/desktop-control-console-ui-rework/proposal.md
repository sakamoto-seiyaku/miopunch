## Why

The desktop frontend prototype has moved from a POC task launcher toward a control console, but the backend desktop runtime contract still lacks several fields the confirmed UI needs for stable live-mode rendering. This change aligns the existing LocalAPI/Wails desktop runtime state with the new `Network / Shell / Admin / Settings` console model without adding a parallel frontend-only API.

## What Changes

- Rework desktop primary navigation around `Network`, `Shell`, `Admin`, and `Settings`; keep `Access` only as a compatibility deep link that redirects into the supported console flow.
- Move invite creation and approval review into `Admin` for owner/admin nodes; first-run and member nodes should land on Network-oriented flows.
- Promote Shell to a first-class workspace backed by the existing `sh_ls`, `sh_attach`, and terminal WebSocket contracts.
- Extend desktop runtime config/preferences with persisted desktop-local peer aliases.
- Extend desktop topology/runtime state with remote member display hints, attachable shell-session status, and optional connection path detail fields.
- Remove live-mode dependency on prototype-only device names, path facts, and metrics; live UI should render only daemon-provided snapshot fields or explicit unknown/not measured states.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: Updates the desktop GUI console navigation, role gating, Shell workspace, alias behavior, and live-mode rendering requirements.
- `miopunch-poc-localapi-v0`: Extends the existing desktop runtime state/config contract for peer aliases, member display metadata, shell attachability, and optional path details.

## Impact

- Affected frontend:
  - `cmd/miopunch-desktop/frontend` navigation, Network detail, Shell workspace, Admin flows, Settings save behavior, and browser tests.
- Affected LocalAPI/Wails contract:
  - `GET /api/v0/desktop/state`
  - `GET /api/v0/desktop/events`
  - `PATCH /api/v0/desktop/config`
  - `SaveDesktopConfig`
- Affected backend runtime state:
  - desktop settings/preferences persistence
  - topology member projection from approved member declarations
  - shell-session summaries and attach lifecycle
  - peer/topology path detail projection where reliable data exists
- Validation impact:
  - OpenSpec-only proposal work does not require the full `$dev` gate.
  - Code implementation will require focused Go tests and frontend browser tests, plus the full `$dev` gate before entering mainline.
