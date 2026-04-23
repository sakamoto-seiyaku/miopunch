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
cd cmd/miopunch-desktop
wails build -clean -platform windows/amd64 -tags desktop -webview2 embed
cp build/bin/miopunch-desktop.exe ../../packaging/windows/nsis/miopunch-desktop.exe
```

## Build installer

```bash
makensis packaging/windows/nsis/miopunch.nsi
```

## Runtime notes

- Install directory: `%ProgramFiles%\\miopunch\\`
- Installer log: `%ProgramData%\\miopunch\\install.log`
- The installer delegates daemon install/uninstall to:
  - `miopunch install-system-daemon` (install: fail-fast)
  - `miopunch uninstall-system-daemon` (uninstall: best-effort)
- The GUI is built with `-webview2 embed` and uses the WebView2 Runtime.
