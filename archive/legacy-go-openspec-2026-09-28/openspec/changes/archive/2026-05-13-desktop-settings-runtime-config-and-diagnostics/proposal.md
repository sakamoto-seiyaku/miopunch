## Why

The desktop shell can display runtime `config` and `diagnostics`, but Settings is
still mostly a LocalAPI override form and raw fact list. Real session-bundle
testing needs daemon-authoritative settings, clear desired/effective runtime
state, recent failure evidence, and a portable diagnostics export before the UI
can be used for repeatable multi-machine troubleshooting.

## What Changes

- Add a minimal Settings runtime config flow for existing fields: MQTT brokers,
  `p2p_network`, `p2p_ip_family`, `data_proto`, `quic_cc`, STUN endpoints,
  portmap/assisted-address toggles, default shell target/session, and log level.
- Extend desktop runtime state so Settings can distinguish desired persisted
  config from effective runtime config and show whether a change is immediate,
  future-connection-only, or needs session reconnect.
- Add a daemon-authoritative LocalAPI write path for desktop config updates with
  structured validation failures and config/diagnostics runtime events.
- Add a desktop diagnostics export that writes a redacted archive through the Go
  bridge, not browser download semantics.
- Update the desktop Settings UI and browser coverage for config editing,
  validation, diagnostics, and export.
- Defer sing-specific settings until the project has a real sing config model.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `miopunch-desktop-gui-v0`: Settings becomes a runtime config and diagnostics
  surface rather than only LocalAPI override/preview controls.
- `miopunch-poc-localapi-v0`: LocalAPI adds a desktop config write route and
  richer desktop runtime config/diagnostics state.
- `miopunch-desktop-packaging-v0`: Session bundles expose exported diagnostics
  from bundle-local logs/data while preserving redaction.

## Impact

- Affected code:
  - daemon task manager desktop state/config assembly;
  - LocalAPI route, client, and error handling;
  - Wails desktop bridge methods and diagnostics archive writer;
  - committed desktop frontend JS and Playwright tests.
- Affected data:
  - existing `data/state.json` remains the source for network settings;
  - new `data/desktop_settings.json` stores desktop-only preferences such as
    default shell target/session and log level.
- Validation:
  - OpenSpec strict validation;
  - focused Go tests for config update, runtime events, and export redaction;
  - Playwright Settings tests;
  - full `$dev` gates before mainline integration.
