## Context

The Android/WSL demo path is already proven at the CLI level: Android arm64 `cmd/miopunch` can run on a Pixel 6a, join a current POC v1 network, see the Linux/WSL peer online, and complete `ping` / `sh ls` with `selected_path=direct_ipv4`. The remaining gap is presentation. For interview use, asking someone to infer the product from ADB or WSL terminal output is weaker than showing the phone itself controlling a remote shell.

The APK must therefore be a thin control surface over the existing binary. It should not create a second runtime model, a HTTP bridge, or a new Android product architecture.

## Goals / Non-Goals

**Goals:**

- Build a minimal Android APK that can be installed on the known arm64 test phone.
- Package the current `miopunch` Android binary in the APK and execute it from the app process.
- Start and stop a local `miopunch up` process for the app lifetime.
- Run `join`, `ls`, `ping`, `sh ls`, and interactive `sh` through child CLI processes using the app-owned LocalAPI socket.
- Show CLI stdout/stderr, action status, and shell output in the phone UI.
- Capture a repeatable Linux/WSL to Android demo runbook and evidence set.

**Non-Goals:**

- Do not make Android a controlled shell target or expose an Android shell to peers.
- Do not add background services, boot receivers, notification-resident daemons, or long-lived Android runtime management.
- Do not implement a new LocalAPI client, HTTP bridge, websocket bridge, or Android-native shell protocol client.
- Do not build a polished Material product UI, QR scanner, roster manager, revoke flow, or full governance console.
- Do not change current POC v1 wire, punch, session, runtime, or LocalAPI contracts.
- Do not make TCP Door-2, relay, or `p2p_network=tcp_only` part of this demo acceptance.

## Decisions

1. **Create a standalone `android/control-lite` demo project in this repo.**

   This keeps the interview demo attached to miopunch while avoiding accidental coupling with the unrelated Sakamoto Android product. The project should be small enough to delete or replace when a formal Android control client starts.

   Alternative considered: implement inside `sakamoto-app`. That would reuse Android infrastructure, but it would blur product ownership and make the demo harder to explain.

2. **Use a simple Android Activity and platform widgets instead of Compose.**

   The first APK only needs forms, buttons, logs, and a text shell console. A programmatic Activity avoids adding Compose, Hilt, navigation, or a design system to a NAT traversal repo just for a demo.

   Alternative considered: Compose UI. It is better for a real app, but adds setup cost and dependencies that do not improve the demo's core proof.

3. **Package `miopunch` as an extracted native payload.**

   The build should cross-compile `cmd/miopunch` with `GOOS=android GOARCH=arm64`, stage it as `lib/arm64-v8a/libmiopunch.so`, set native library extraction on, and execute the extracted file from `applicationInfo.nativeLibraryDir`.

   Alternative considered: put the binary in assets and copy it into app-private storage. Recent Android versions restrict execution from writable app data; the extracted native library path is the more reliable first path.

4. **Keep all backend operations as CLI subprocesses.**

   The app starts `miopunch up` as a long-running process, then starts short-lived child processes for `join`, `ls`, `ping`, and `sh ls`. For interactive shell, it starts `miopunch sh` and wires UI input to stdin while streaming stdout/stderr back into the shell/log views.

   Alternative considered: implement LocalAPI and shell stream clients in Android. That is the right long-term architecture, but it duplicates protocol code before the demo proves value.

5. **Use app-private paths and app-lifetime cleanup.**

   Runtime state lives under `filesDir/state/state.json`; LocalAPI uses `unix:<cacheDir>/miopunch-localapi.sock`; logs live under `filesDir/logs/`. `Stop`, Activity destruction, and explicit restart should terminate runtime and shell subprocesses and remove stale socket files when possible.

## Risks / Trade-offs

- Native payload execution can vary by Android version -> use `nativeLibraryDir` extraction first and document the exact device/SDK evidence; fall back to ADB/Termux only as a manual debugging path, not the primary demo.
- `miopunch up` may still require `su` on some path or socket combinations -> keep app paths private and validate without `su`; if the known device still requires root, document that as a demo precondition before widening scope.
- A raw text-area shell cannot render tmux/zsh control sequences -> bundle xterm.js locally and render shell stdout in a WebView while keeping the backend path as the same `miopunch sh` subprocess.
- Runtime subprocesses can outlive the Activity if cleanup is wrong -> centralize process ownership in one app controller and add explicit Stop behavior before polishing UI.
- Existing dirty repo state includes prior direct-first changes -> this change consumes that baseline and must not revert unrelated work.

## Migration Plan

Implement as an isolated demo addition. No persisted miopunch state migration, broker migration, protocol migration, or existing product entrypoint migration is required.

Rollback is to remove `android/control-lite`, the demo scripts, and the associated OpenSpec change; existing CLI and POC v1 runtime behavior remain unchanged.

## Open Questions

- None for the first demo. A formal Android control client, Material UI, QR invite flow, and native LocalAPI client should be separate follow-up changes after the APK proves the shell demo.
