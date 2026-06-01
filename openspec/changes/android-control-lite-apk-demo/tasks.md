## 1. Android Demo Project Scaffold

- [x] 1.1 Add an isolated `android/control-lite` Android application project with a single launcher Activity and no dependency on desktop GUI code.
- [x] 1.2 Configure the project for `arm64-v8a`, app-private native library extraction, and a debug APK build path.
- [x] 1.3 Add a build/staging script that cross-compiles `cmd/miopunch` for `GOOS=android GOARCH=arm64` and stages it as `lib/arm64-v8a/libmiopunch.so`.
- [x] 1.4 Add a small WSL-friendly install/run helper or document using the existing working ADB wrapper pattern for installing the debug APK on the Pixel 6a.

## 2. APK Runtime Process Control

- [x] 2.1 Resolve the packaged `miopunch` executable path from `applicationInfo.nativeLibraryDir` and verify it can run `--help`.
- [x] 2.2 Implement `Start Runtime` to launch `miopunch up --localapi unix:<cacheDir>/miopunch-localapi.sock --state_path <filesDir>/state/state.json --log-level trace`.
- [x] 2.3 Stream runtime stdout/stderr into the app log view and persist a copy under `<filesDir>/logs/`.
- [x] 2.4 Implement `Stop` and Activity cleanup to terminate the runtime process, active shell process, and stale app-owned LocalAPI socket when possible.

## 3. APK Control Actions

- [x] 3.1 Build the minimal UI fields for invite code, peer ID, target defaulting to `local`, and session defaulting to `main`.
- [x] 3.2 Implement `Join` by running `miopunch --localapi <app-localapi> join <invite_code>` and appending stdout/stderr to the log view.
- [x] 3.3 Implement `LS`, `Ping`, and `Shell LS` by running the packaged CLI against the app LocalAPI and preserving CLI output, `reason_code`, and selected path facts.
- [x] 3.4 Keep action buttons disabled or visibly rejected when the runtime is stopped or required inputs are missing.

## 4. Simple Interactive Shell

- [x] 4.1 Implement `Open Shell` by starting `miopunch --localapi <app-localapi> sh <peer_id> <target> -s <session>`.
- [x] 4.2 Stream shell stdout to a shell output view and shell stderr to the log view.
- [x] 4.3 Add a line-oriented shell input that writes submitted commands plus carriage return to the shell subprocess stdin.
- [x] 4.4 Handle shell subprocess exit by updating the visible shell state and leaving previous shell output available for demo evidence.

## 5. Demo Runbook and Evidence

- [x] 5.1 Add a short runbook for the Linux/WSL peer setup, broker setup, APK build/install, invite/join/approve sequence, and phone-side shell demo.
- [x] 5.2 Document the explicit non-goals: Android is control-only, no Android shell target, no background daemon, no HTTP bridge, no TCP Door-2 acceptance.
- [x] 5.3 Capture the expected evidence files: Android app/runtime logs, Linux daemon logs, successful `ping` / `sh ls` facts, and successful shell command output.

## 6. Validation

- [x] 6.1 Build the Android arm64 `miopunch` payload from the current tree.
- [x] 6.2 Assemble the Android control-lite debug APK.
- [x] 6.3 Install the APK on the Pixel 6a and verify the packaged payload can run `miopunch --help`.
- [x] 6.4 Validate `Start Runtime -> Join -> LS -> Ping -> Shell LS -> Open Shell` against the Linux/WSL peer.
- [x] 6.5 In the phone shell UI, run `date`, `whoami`, `pwd`, and `ls`, then save the logs/evidence path in the runbook or change notes.
