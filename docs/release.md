# Release Automation

This repository publishes GitHub Releases from maintainer-created `v*` tags.
The release workflow does not create or move tags.

## Current POC Release

The current POC release tag is:

```bash
git tag -a v0.0.3 -m "miopunch v0.0.3"
git push origin v0.0.3
```

The local `origin` remote may be a mirror endpoint. The release workflow runs
after the tag reaches GitHub.

## Workflows

- `Go Checks`: host Go gates and release cross-build sanity.
- `Build Artifacts`: Linux session bundle, Windows session bundle, and Android
  control-lite debug APK.
- `Release`: tag-triggered orchestration that publishes only after Go checks
  and artifact builds pass.

The old VM/lab workflows were removed from CI for the POC release path. Lab
scripts may still exist locally as reference/debug tooling, but GitHub release
publishing no longer waits for VM-based gates.

## Local Build Helpers

```bash
MIOPUNCH_VERSION=v0.0.3 bash scripts/release/build_bundles.sh
MIOPUNCH_VERSION=v0.0.3 bash scripts/release/build_android_control_lite.sh
bash scripts/release/generate_manifest.sh dist/release
```

By default, `build_bundles.sh` emits only the current portable session bundles.
Set `MIOPUNCH_BUILD_LEGACY_BUNDLES=1` only when older CLI helper archives are
explicitly needed. The retired `miopunch-lab` command is archived out of the
active Go module and is not part of the current release build.

GitHub CI and the tag-driven `Release` workflow follow this same default path.
They do not enable `MIOPUNCH_BUILD_LEGACY_BUNDLES=1`, and they do not build the
deferred `.deb` or NSIS packaging routes.

The final release asset directory must include `checksums.txt` and
`release-manifest.json`.

Current desktop smoke uses portable session bundles:

- `miopunch_<version>_windows_amd64_session.zip`
- `miopunch_<version>_linux_amd64_session.tar.gz`

The current GitHub Release asset contract is exactly:

- the Windows session bundle
- the Linux session bundle
- the Android arm64 control-lite debug APK
- `checksums.txt`
- `release-manifest.json`

Desktop release binaries use Wails production build tags. Windows GUI release
builds use `desktop,production,wv2runtime.embed` and `-H windowsgui`; Linux GUI
release builds use `desktop,production`.

NSIS and `.deb` packaging remain deferred `D1a-privileged` scaffolding. They
are not the current session smoke gate and are not required for release-candidate
desktop smoke.

The Android APK is intentionally debug-signed. It is a demo control client that
packages the Android arm64 `miopunch` CLI payload and is not the formal Android
product line.

## Local Windows Session Smoke

The Windows desktop session artifact is not complete until it has been smoke
tested on a local Windows machine as a normal user.

Build the session bundle from this repository or use the matching CI artifact:

```bash
MIOPUNCH_VERSION=v0.0.3 bash scripts/release/build_bundles.sh
```

Copy `dist/release/miopunch_v0.0.3_windows_amd64_session.zip` to Windows
and extract it. From a normal PowerShell session in the extracted directory:

```powershell
.\miopunch-desktop.exe
```

Smoke checklist:

- The extracted directory contains `miopunch.exe`, `miopunch-desktop.exe`, and smoke instructions.
- Launching `miopunch-desktop.exe` starts or reuses sibling `miopunch.exe`.
- GUI connects to LocalAPI and reports the selected endpoint and daemon ownership.
- Core desktop task flow works without an Administrator prompt.
