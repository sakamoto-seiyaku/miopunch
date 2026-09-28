# miopunch

miopunch is currently a POC for peer-to-peer remote control over a punched UDP
path. The present goal is an interview/demo-ready system, not a polished product
or installer.

The validated POC surface is:

- portable Linux and Windows desktop session bundles
- a desktop GUI that starts or reuses the bundled `miopunch` runtime
- CLI/runtime commands for network init, invite/join/approve, peer discovery,
  ping, and remote shell
- an Android control-lite debug APK that can join the POC network, ping a
  Linux/WSL peer, and open the remote shell from a phone

## Release Artifacts

The current release tag is `v0.0.3`. The release workflow publishes:

- `miopunch_v0.0.3_linux_amd64_session.tar.gz`
- `miopunch_v0.0.3_windows_amd64_session.zip`
- `miopunch_v0.0.3_android_arm64_control-lite-debug.apk`
- `checksums.txt`
- `release-manifest.json`

## Local Checks

```bash
export PATH=/usr/local/go/bin:$PATH
go test ./...
go vet ./...
bash scripts/check_no_xtcp_imports.sh
```

Desktop UI smoke:

```bash
cd cmd/miopunch-desktop/frontend
npm test
```

Build release artifacts locally:

```bash
MIOPUNCH_VERSION=v0.0.3 bash scripts/release/build_bundles.sh
MIOPUNCH_VERSION=v0.0.3 bash scripts/release/build_android_control_lite.sh
bash scripts/release/generate_manifest.sh dist/release
```

Android build requirements are documented in `android/control-lite/README.md`.
