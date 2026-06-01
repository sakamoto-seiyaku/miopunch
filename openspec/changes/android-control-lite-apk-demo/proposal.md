## Why

Android arm64 CLI has already been proven to execute on a real Pixel 6a, join a current POC v1 network, discover a Linux/WSL peer, and complete `ping` / `sh ls` over `direct_ipv4`. The missing interview demo surface is a phone-native control shell that can show a real remote Linux shell without asking the interviewer to read terminal output from a laptop.

This change turns that proven CLI path into the smallest Android APK demo while keeping the Android device as a control-only operator, not a controlled shell target.

## What Changes

- Add an isolated Android control-lite APK under the miopunch repo for demo use.
- Package the current `cmd/miopunch` Android arm64 executable as the APK's native payload.
- Let the APK start and stop a local, app-lifetime `miopunch up` process using app-private `localapi`, state, and log paths.
- Let the APK execute `join`, `ls`, `ping`, `sh ls`, and interactive `sh` by launching the packaged `miopunch` binary with the app-owned LocalAPI endpoint.
- Provide a minimal phone UI for invite code, peer ID, target, session name, action buttons, logs, and an xterm-backed shell console.
- Add a demo runbook that fixes the Linux/WSL peer setup, Android install path, and evidence capture steps.
- Keep Android control-lite separate from the desktop GUI and from the formal Android product line.

## Capabilities

### New Capabilities

- `miopunch-android-control-lite-apk`: A minimal Android control-only APK demo that packages the existing CLI runtime and opens an interactive remote shell from the phone.

### Modified Capabilities

- None.

## Impact

- Affected areas: new Android demo project files, Android binary staging/build scripts, and demo documentation.
- Existing Go runtime, LocalAPI, punch/session behavior, and wire contracts are consumed as-is.
- No breaking CLI syntax changes.
- No new network protocol or broker dependency.
- Validation focuses on Android APK build/install plus real Pixel 6a to Linux/WSL `join -> ping -> sh` evidence; full mainline gates remain out of scope for this OpenSpec-only proposal.
