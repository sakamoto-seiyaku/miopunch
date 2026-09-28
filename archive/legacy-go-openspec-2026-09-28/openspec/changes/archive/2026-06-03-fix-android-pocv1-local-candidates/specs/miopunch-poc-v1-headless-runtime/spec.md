## ADDED Requirements

### Requirement: Android candidate diagnostics are stage-locatable
The current v1 runtime SHALL expose trace diagnostics that identify Android
local candidate sources, candidate counts, and candidate filtering outcomes
during peer session establishment.

Diagnostics SHALL distinguish at least Android provider enumeration,
route-source derivation, STUN mapped address gathering, direct path selection,
UDP punching fallback, and secure-session upgrade.

#### Scenario: Android direct candidates are visible in logs
- **WHEN** Android Control Lite starts the current v1 runtime with trace logging
- **AND** it attempts a P2P action against a Linux peer
- **THEN** logs identify the final Android direct candidate set and its source

#### Scenario: Downstream secure session failure preserves candidate evidence
- **WHEN** P2P path establishment succeeds but secure-session upgrade fails
- **THEN** logs preserve the selected path, selected endpoints, candidate source
  evidence, and secure-session failure stage

### Requirement: Android Control Lite validation uses rebuilt shared runtime
Android Control Lite validation SHALL rebuild the APK payload and the Linux CLI
from the same source tree before judging Android/Linux P2P behavior.

Validation SHALL use fresh app data, fresh Linux state, trace logs on both sides,
and SHALL verify Android-to-Linux `ping` plus `sh ls` when Android remains a
control-side demo client.

#### Scenario: Rebuilt Android demo proves P2P path
- **WHEN** the Android APK and Linux CLI are rebuilt from the changed tree
- **AND** the app joins the same network as the Linux peer using fresh state
- **THEN** Android-to-Linux `ping` succeeds or logs enough stage evidence to
  identify the failing layer
- **AND** Android-to-Linux `sh ls` is run as the shell demo acceptance check
