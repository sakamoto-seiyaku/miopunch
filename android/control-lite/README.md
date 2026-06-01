# Miopunch Android Control Lite

This is a deliberately small Android demo shell for `cmd/miopunch`.

It packages the Android arm64 `miopunch` executable as an extracted native
payload and drives the existing CLI through child processes. It is not the
formal Android product line.

The current validated demo path is:

```text
Pixel 6a APK -> app-owned miopunch up -> Linux/WSL peer ping -> remote shell
```

The shell pane uses a bundled xterm.js WebView so ANSI/tmux/zsh output renders
as a terminal instead of raw escape sequences.

## Build

```bash
bash android/control-lite/scripts/build-debug-apk.sh
```

The script expects:

- Go at `/usr/local/go/bin/go` or in `PATH`
- JDK tools (`javac`, `keytool`)
- Android SDK build tools, either from `ANDROID_SDK_ROOT` / `ANDROID_HOME` or
  from `/mnt/c/Android/SDK` in the current WSL setup

Output:

```text
android/control-lite/build/outputs/miopunch-control-lite-debug.apk
```

## Install

```bash
bash android/control-lite/scripts/install-debug-apk.sh
```

Set `MIOPUNCH_ADB=/path/to/adb` to override ADB discovery.

## Scope

- Android is the control side only.
- The APK does not expose Android as a remote shell target.
- The APK starts a local `miopunch up` process only for the app lifetime.
- Control actions and shell attach are still the existing CLI behavior.

## Confirmed Evidence

2026-05-31 real-device validation passed on Pixel 6a against a Linux/WSL peer:

- `Ping` returned `reason_code=OK` and `selected_path=direct_ipv4`.
- `Open Shell` rendered the remote tmux/zsh shell in the in-app terminal.
- Sending `whoami` from the phone returned `js`.

Evidence screenshots:

```text
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-ping-ok.png
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-shell-whoami-js.png
```
