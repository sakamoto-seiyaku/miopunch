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
- `--webkit2_41`: build with `-tags webkit2_41` and depend on WebKitGTK 4.1

## Build (local)

```bash
./packaging/linux/deb/build_deb.sh          # default (WebKitGTK 4.0)
./packaging/linux/deb/build_deb.sh --all    # both variants
```

If you already have prebuilt binaries, you can bypass Go builds:

```bash
MIOPUNCH_BIN=/path/to/miopunch \
MIOPUNCH_DESKTOP_BIN=/path/to/miopunch-desktop \
  ./packaging/linux/deb/build_deb.sh
```
