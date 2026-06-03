## Why

Android Control Lite can start the shared Go runtime, join the POC v1 network, and
exchange control-plane messages, but targetSdk 30+ Android rejects the current
`net.Interfaces()` / `net.InterfaceAddrs()` local-address path with
`netlinkrib: permission denied`. As a result Android publishes no usable direct
candidates and P2P path establishment fails downstream during secure-session
upgrade.

## What Changes

- Add an Android-specific Go local candidate provider inside the shared runtime.
- Avoid patching Go's standard library and avoid moving candidate collection to
  Android Java/Kotlin.
- Add route-source candidate derivation from peer direct targets as a fallback
  and supplement to Android interface enumeration.
- Add trace diagnostics that prove which local-address source was used and why a
  candidate set is empty.
- Validate with a rebuilt Android APK and Linux CLI from the same tree.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `miopunch-poc-v1-dial-punch`: current v1 candidate gathering must support
  Android app sandboxes without relying on Go standard interface enumeration.
- `miopunch-poc-v1-headless-runtime`: current v1 runtime diagnostics and Android
  demo validation must expose enough evidence to prove Android local candidates
  and P2P path establishment.

## Impact

- Affected implementation areas: `connectivity`, `internal/pocv1/runtime`, and
  current POC v1 punch/session diagnostics.
- Affected validation: focused Go tests plus real Android Control Lite APK
  install/run against a Linux peer with trace logs.
- No wire-format change, no broker behavior change, no TCP punching restoration,
  no relay, and no Android Java/Kotlin local-address collection requirement.
