# Android Control Lite Demo Runbook

This runbook is for the current interview demo path:

```text
Pixel 6a Android APK
-> local app-lifetime miopunch up
-> join current POC v1 network
-> ping Linux/WSL peer
-> open Linux/WSL shell target from the phone
```

Android is the operator/control side only. It is not a controlled shell target.

## Current Confirmed Result

2026-05-31 validation passed on Pixel 6a with a Linux/WSL peer:

- The APK-installed payload runs `miopunch --help`.
- `Start Runtime` starts the app-owned `miopunch up` process without `su`.
- `Ping` from Android to Linux/WSL returns `reason_code=OK` and `selected_path=direct_ipv4`.
- `Open Shell` opens the Linux/WSL shell in the phone UI.
- Sending `whoami` from the phone returns `js`.

Evidence screenshots from the confirmed run:

```text
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-ping-ok.png
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-shell-whoami-js.png
```

## References

- Android ABI packaging: <https://developer.android.com/ndk/guides/abis.html>
- `<application android:extractNativeLibs>`: <https://developer.android.com/guide/topics/manifest/application-element>
- AAPT2 command-line packaging: <https://developer.android.com/studio/command-line/aapt2>
- D8 command-line dexing: <https://developer.android.com/tools/d8>
- APK signing with `apksigner`: <https://developer.android.com/tools/apksigner>

## Build The APK

From the repo root:

```bash
export PATH=/usr/local/go/bin:$PATH
bash android/control-lite/scripts/build-debug-apk.sh
```

Expected output:

```text
android/control-lite/build/outputs/miopunch-control-lite-debug.apk
```

The build script:

- cross-compiles `cmd/miopunch` with `GOOS=android GOARCH=arm64`
- stages it as `lib/arm64-v8a/libmiopunch.so`
- builds a small Java Activity
- uses `aapt2`, `d8`, `zipalign`, and `apksigner` to create a debug APK

## Install On Pixel 6a

```bash
bash android/control-lite/scripts/install-debug-apk.sh
```

If needed, override ADB:

```bash
MIOPUNCH_ADB=/path/to/adb bash android/control-lite/scripts/install-debug-apk.sh
```

## Fast Phone Smoke Check

After installing, launch the app and verify the packaged payload starts:

```bash
/home/js/Git/sakamoto/sakamoto-app/scripts/adb shell am force-stop com.miopunch.controlite
/home/js/Git/sakamoto/sakamoto-app/scripts/adb logcat -c
/home/js/Git/sakamoto/sakamoto-app/scripts/adb shell am start -n com.miopunch.controlite/.MainActivity
sleep 2
/home/js/Git/sakamoto/sakamoto-app/scripts/adb logcat -d -s MiopunchControlLite '*:S'
```

Expected log line:

```text
payload check exit=0
```

Tap `Start Runtime`; the app log should show `miopunch.so up ... --log-level trace`.
Tap `Stop`; the app log should show `runtime exited rc=0`.

## Prepare The Linux/WSL Peer

Use the current tree for both Linux and Android payloads.

Start a broker reachable by the Android demo. For same-host WSL testing with ADB
reverse, a local broker is enough:

```bash
docker run --rm -p 18883:1883 eclipse-mosquitto:2
adb reverse tcp:18883 tcp:18883
```

Start the Linux daemon with trace logs and a stable state path:

```bash
mkdir -p /tmp/miopunch-control-lite-demo/linux
go build -trimpath -o /tmp/miopunch-control-lite-demo/miopunch ./cmd/miopunch

/tmp/miopunch-control-lite-demo/miopunch up \
  --localapi unix:/tmp/miopunch-control-lite-demo/linux/localapi.sock \
  --broker tcp://127.0.0.1:18883 \
  --state_path /tmp/miopunch-control-lite-demo/linux/state.json \
  --log-level trace \
  > /tmp/miopunch-control-lite-demo/linux/miopunch.stdout.log \
  2> /tmp/miopunch-control-lite-demo/linux/miopunch.stderr.log
```

In another terminal:

```bash
M=/tmp/miopunch-control-lite-demo/miopunch
API=unix:/tmp/miopunch-control-lite-demo/linux/localapi.sock

"$M" --localapi "$API" --format json --redact init-network --new --confirm create-new-network
"$M" --localapi "$API" --format json invite --mode approve --uses 1 --expires 30m
```

Copy the invite code into the phone UI.

Before tapping Android `Join`, start approve from Linux and leave it waiting:

```bash
"$M" --localapi "$API" --format json --redact approve "<invite_code>"
```

Then tap Android `Join`. After approve returns, confirm the Linux roster:

```bash
"$M" --localapi "$API" --format json --redact ls
```

Use the Linux peer ID shown by the phone `LS` output in the phone UI.

## Phone Demo Sequence

Open `Miopunch Control Lite` on the phone:

1. `Start Runtime`
2. Paste invite code
3. Start Linux `approve` and leave it waiting
4. Tap `Join`
5. Tap `LS`
6. Enter the Linux peer ID
7. Keep `P2P path` as `auto / auto` for the normal demo, or select a diagnostic
   override such as `udp_only / v4` before P2P actions
8. Tap `Ping`
9. Tap `Shell LS`
10. Tap `Open Shell`
11. Send these commands from the phone:

```text
date
whoami
pwd
ls
```

Successful `Ping` / `Shell LS` output should include `reason_code=OK`. On the
same-LAN Android/WSL path, the expected selected path is `selected_path=direct_ipv4`.
The `P2P path` selectors affect only `Ping`, `Shell LS`, and `Open Shell`.
They do not constrain runtime startup, invite/join/approve, roster `LS`, MQTT
signaling, or STUN discovery. Current POC v1 exposes `tcp_only` as a diagnostic
choice but rejects it explicitly because TCP punching is not implemented.

For repeatable ADB-driven evidence, the debug Activity accepts intent extras that
prefill UI fields and the same path controls used by the buttons:

```bash
adb shell am start -n com.miopunch.controlite/.MainActivity --es invite "<invite_code>"
adb shell am start -n com.miopunch.controlite/.MainActivity --es peer "<linux_peer_id>"
adb shell am start -n com.miopunch.controlite/.MainActivity --es p2p_network udp_only --es p2p_ip_family v4
adb shell am start -n com.miopunch.controlite/.MainActivity --es line date
```

The `line` extra writes through the same shell input path as the `Send` button.

The shell surface is a bundled xterm.js WebView. It is still driven by the
existing `miopunch sh` subprocess, but it renders tmux/zsh ANSI output correctly
for demo commands.

## Evidence To Save

Save these after a successful run:

- Android app log surface copied from the phone, or `adb logcat` if needed.
- Android app files under `<filesDir>/logs/` when pulled during debugging.
- Linux daemon stdout/stderr logs.
- Linux CLI JSON for `init-network`, `invite`, `approve`, and `ls`.
- Phone-visible `ping`, `sh ls`, and shell command output.

Suggested evidence root:

```text
/tmp/miopunch-control-lite-demo/evidence/
```

## Explicit Non-Goals

- No Android controlled shell target.
- No Android background daemon, boot receiver, or notification-resident service.
- No HTTP bridge, websocket bridge, or Android-native LocalAPI client.
- No QR scanner, roster manager, revoke UI, or polished Material product flow.
- No TCP Door-2 acceptance and no `p2p_network=tcp_only` demo claim.
