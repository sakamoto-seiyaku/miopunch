# Windows installer (NSIS) — `miopunch`

This directory contains the minimal NSIS installer script for the Door 1 desktop
shell.

## Build inputs

The NSIS script expects these files to exist **next to** `miopunch.nsi` at
compile time:

- `miopunch.exe`
- `miopunch-desktop.exe`

## Build binaries (example)

Build `miopunch.exe`:

```bash
GOOS=windows GOARCH=amd64 go build -o packaging/windows/nsis/miopunch.exe ./cmd/miopunch
```

Build `miopunch-desktop.exe` (Wails / WebView2 embed):

```bash
GOOS=windows GOARCH=amd64 go build -trimpath \
  -tags desktop,production,wv2runtime.embed \
  -ldflags "-s -w -H windowsgui" \
  -o packaging/windows/nsis/miopunch-desktop.exe \
  ./cmd/miopunch-desktop
```

## Build installer

```bash
makensis packaging/windows/nsis/miopunch.nsi
```

The CI release path uses the repository helper, which builds the required
Windows binaries in a temporary NSIS work directory and writes the installer to
`dist/release/`:

```bash
MIOPUNCH_VERSION=v0.1.0-rc.1 bash scripts/release/build_windows_installer.sh
```

## Runtime notes

- Install directory: `%ProgramFiles%\\miopunch\\`
- Installer log: `%ProgramData%\\miopunch\\install.log`
- Windows uninstall entry: Apps & Features / Programs and Features
- Start menu uninstall shortcut: `miopunch\\Uninstall miopunch`
- The installer delegates daemon install/uninstall to:
  - `miopunch install-system-daemon` (install: fail-fast)
  - `miopunch uninstall-system-daemon` (uninstall: best-effort)
- The GUI is built with Wails production tags and `wv2runtime.embed`, equivalent
  to Wails' WebView2 embed strategy.

## Local smoke

Before considering Windows packaging complete, install the generated setup
executable on a local Windows machine as Administrator. Verify that the service
is installed and running, the Start menu shortcut launches `miopunch-desktop.exe`
without the Wails build-tags dialog, the GUI connects to LocalAPI, Apps &
Features shows the uninstall entry, and uninstall removes the installed binaries
and shortcuts.
