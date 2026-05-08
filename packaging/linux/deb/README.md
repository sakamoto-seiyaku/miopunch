# Linux `.deb` packaging (v0) — `miopunch`

This directory contains a minimal `.deb` packaging scaffold for the Door 1
desktop shell.

## Contract (paths)

- `/usr/bin/miopunch` (CLI + daemon)
- `/usr/bin/miopunch-desktop` (GUI)
- `/usr/share/applications/miopunch.desktop` (Exec: `miopunch-desktop`)
- `/usr/share/icons/hicolor/scalable/apps/miopunch.svg`

## Installer scripts

- `postinst`: calls `miopunch install-system-daemon` (fail-fast), appends logs to
  `/var/log/miopunch/install.log`, and prints operator-group instructions.
- `prerm`: calls `miopunch uninstall-system-daemon` (continue only on
  ExitCodeNotFound=7; otherwise fail-fast) and appends logs to
  `/var/log/miopunch/install.log`.
- `postrm`: on `purge`, removes `/var/lib/miopunch` and `/var/log/miopunch`.

## Variants

Two `.deb` variants are supported:

- default: WebKitGTK 4.0 runtime dependency
- `--webkit2_41`: add the `webkit2_41` tag to the Wails
  `desktop,production` build and depend on WebKitGTK 4.1

Packages are built with `xz` compression so older `dpkg` releases, including
Debian 11, can inspect and install the metadata.

## Build (local)

Use the Docker wrapper when producing release-like packages. It builds each
variant inside the distro that provides the matching WebKitGTK development
stack, and it is the same entrypoint used by CI.

```bash
./packaging/linux/deb/build_deb_docker.sh --webkit2_40  # ubuntu:22.04 builder
./packaging/linux/deb/build_deb_docker.sh --webkit2_41  # ubuntu:24.04 builder
./packaging/linux/deb/build_deb_docker.sh --all         # both variants
```

Add `--smoke-install` to run an apt resolver check in the target Ubuntu
container after the package is built.

```bash
./packaging/linux/deb/build_deb_docker.sh --all --smoke-install
```

The native builder remains available for hosts that already have the matching
development packages installed:

```bash
./packaging/linux/deb/build_deb.sh             # default (WebKitGTK 4.0)
./packaging/linux/deb/build_deb.sh --webkit2_41
```

On Debian 11, the native builder can usually build the WebKitGTK 4.0 package
but not the WebKitGTK 4.1 package because `webkit2gtk-4.1` and `libsoup-3.0`
development files are not available from the base distro.

If you already have prebuilt binaries, you can bypass Go builds:

```bash
MIOPUNCH_BIN=/path/to/miopunch \
MIOPUNCH_DESKTOP_BIN=/path/to/miopunch-desktop \
  ./packaging/linux/deb/build_deb.sh
```

Release builds can pass the public tag. The script converts release candidates
to Debian ordering, for example `v0.1.0-rc.1` becomes `0.1.0~rc.1`.

```bash
MIOPUNCH_VERSION=v0.1.0-rc.1 ./packaging/linux/deb/build_deb_docker.sh --all
```

## Install target

- Ubuntu 22.04 / Debian 11-compatible desktops: install the default
  `miopunch_<version>_amd64.deb` WebKitGTK 4.0 package.
- Ubuntu 24.04 and newer desktops: install the
  `miopunch_<version>+webkit2.41_webkit2_41_amd64.deb` WebKitGTK 4.1 package.
