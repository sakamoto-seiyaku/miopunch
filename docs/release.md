# Release Automation

This repository publishes GitHub Releases from maintainer-created `v*` tags.
The release workflow does not create or move tags.

## First Release Candidate

The first planned public candidate is:

```bash
git tag -a v0.1.0-rc.1 -m "miopunch v0.1.0-rc.1"
git push origin v0.1.0-rc.1
```

The local `origin` remote may be a mirror endpoint. The release workflow runs
after the tag reaches GitHub.

## Workflows

- `Go Checks`: host Go gates and release cross-build sanity.
- `Build Artifacts`: pure artifact build without publishing a release.
- `Lab Core Gates`: `selftest`, `xtcp-selftest`,
  `xtcp-connectivity-selftest`, and `xtcp-fulltest`.
- `Lab Scenario Gates`: scenario 1 (`mnt01-fulltest`), scenario 2
  (`mnt02-selftest`), and scenario 3 (`mnt03-fulltest`). This workflow is
  retained for manual diagnosis and is not a `Release` workflow dependency.
- `Release`: tag-triggered orchestration that publishes only after Go checks,
  artifact builds, and core lab gates pass.

## Hosted Lab Runner Constraint

The v0 release-blocking core lab gates use GitHub-hosted Ubuntu runners. The
lab harness falls back to QEMU TCG when `/dev/kvm` is unavailable, which can be
much slower than a dedicated KVM runner. A core lab timeout or failure still
blocks release publishing, and the workflows upload `lab/_artifacts/` plus QEMU
logs for diagnosis.

Scenario 1/2/3 gates are not release workflow dependencies because they are not
reliable on GitHub-hosted runners. Run them locally before pushing a release
tag:

```bash
./lab/host/labctl mnt01-fulltest
./lab/host/labctl mnt02-selftest
./lab/host/labctl mnt03-fulltest
```

## Local Build Helpers

```bash
MIOPUNCH_VERSION=v0.1.0-rc.1 bash scripts/release/build_bundles.sh
bash scripts/release/generate_manifest.sh dist/release
```

By default, `build_bundles.sh` emits only the current portable session bundles.
Set `MIOPUNCH_BUILD_LEGACY_BUNDLES=1` only when the older CLI/lab binary archives
are explicitly needed.

The final release asset directory must include `checksums.txt` and
`release-manifest.json`.

Current desktop smoke uses portable session bundles:

- `miopunch_<version>_windows_amd64_session.zip`
- `miopunch_<version>_linux_amd64_session.tar.gz`

Desktop release binaries use Wails production build tags. Windows GUI release
builds use `desktop,production,wv2runtime.embed` and `-H windowsgui`; Linux GUI
release builds use `desktop,production`.

NSIS and `.deb` packaging remain deferred `D1a-privileged` scaffolding. They
are not the current session smoke gate and are not required for release-candidate
desktop smoke.

## Local Windows Session Smoke

The Windows desktop session artifact is not complete until it has been smoke
tested on a local Windows machine as a normal user.

Build the session bundle from this repository or use the matching CI artifact:

```bash
MIOPUNCH_VERSION=v0.1.0-rc.1 bash scripts/release/build_bundles.sh
```

Copy `dist/release/miopunch_v0.1.0-rc.1_windows_amd64_session.zip` to Windows
and extract it. From a normal PowerShell session in the extracted directory:

```powershell
.\miopunch-desktop.exe
```

Smoke checklist:

- The extracted directory contains `miopunch.exe`, `miopunch-desktop.exe`, and smoke instructions.
- Launching `miopunch-desktop.exe` starts or reuses sibling `miopunch.exe`.
- GUI connects to LocalAPI and reports the selected endpoint and daemon ownership.
- Core desktop task flow works without an Administrator prompt.
