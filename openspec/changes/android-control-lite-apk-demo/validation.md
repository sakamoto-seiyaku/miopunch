# Validation

Date: 2026-05-31

Environment:

- Android device: Pixel 6a (`28201JEGR0XPAJ`)
- Android package: `com.miopunch.controlite`
- Linux/WSL peer binary: `/tmp/miopunch-control-lite-demo-final/miopunch`
- Broker: Docker `eclipse-mosquitto:2` on `127.0.0.1:18883`
- Android broker reachability: `adb reverse tcp:18883 tcp:18883`

Final evidence root:

```text
/tmp/miopunch-control-lite-demo-final/evidence/
```

Results:

- APK build and install passed.
- Android UI smoke passed after the in-app log refinement: action output is visible in the `Logs` area without requiring ADB logcat.
- Android shell rendering smoke passed after the xterm.js refinement: the APK packages local `assets/terminal/index.html` plus xterm.js/css, and the phone UI shows an `android.webkit.WebView` terminal surface instead of the raw `TextView` shell pane.
- Android packaged payload `--help` check passed.
- Linux `init-network` passed with `broker_endpoint=tcp://127.0.0.1:18883`.
- Android `Start Runtime -> Join` passed after Linux `approve` was already waiting.
- Android UI `LS` passed and showed one online Linux peer.
- Android UI `Ping` passed with `reason_code=OK` and `selected_path=direct_ipv4`.
- Android UI `Shell LS` passed with `reason_code=OK`, `target=local`, `session=main`, and `selected_path=direct_ipv4`.
- Android opened a remote shell to the Linux peer and sent `date`, `whoami`, `pwd`, and `ls`.
- Shell output showed `whoami=js`, `pwd=/home/js/Git/miopunch`, and repository directory entries.
- Latest UI smoke artifacts: `/tmp/miopunch-control-lite-window-no-status.xml`, `/tmp/miopunch-control-lite-window-start-no-status.xml`, and `/tmp/miopunch-control-lite-window-missing-peer-no-status.xml`.
- Latest xterm smoke artifact: `/tmp/miopunch-xterm-after-whoami.png`.
- Retested after APK reinstall and local Linux peer restart:
  - Android `Ping` passed with `reason_code=OK` and `selected_path=direct_ipv4`.
  - Android `Open Shell` passed and rendered the remote tmux/zsh screen in the in-app xterm WebView.
  - Android sent `whoami` through the same app shell-send path and the terminal displayed `js`.
  - Evidence screenshots: `/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-ping-ok.png` and `/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-shell-whoami-js.png`.

Notes:

- Linux `approve` must start before tapping Android `Join`; starting approve after Android publishes the join request can miss the non-retained request.
- App-private runtime metadata is still persisted by the runtime, but this demo APK does not render a separate top network/status summary.
- The xterm.js change fixes terminal escape-sequence rendering and now resizes xterm rows to the visible WebView height so new shell output is not clipped below the pane.
- ADB text input corrupted long invite strings through the device input method. The debug APK now accepts `am start --es invite <code>` and `am start --es peer <peer_id>` to prefill fields without bypassing UI actions.
- The debug APK also accepts `am start --es line <command>` for repeatable shell evidence; it calls the same app `sendShellLine()` path.
- Interactive shell input must send carriage return (`\r`) to the remote tmux/PTY, not line feed (`\n`).
