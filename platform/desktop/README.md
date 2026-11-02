# Minimal Tauri platform host

This is the installed desktop lifecycle shell for the existing Rust node.
Its small bundled page is a technical verification surface; the shared Vue
product UI remains outside the current implementation scope.

The process owns `ClientHost`, independently of its WebView. Closing the last
window keeps the node running. Launching the application again asks the existing
instance to reopen its window. Explicit quit closes the SDK before allowing the
process to exit. Native commands run on bounded Rust-owned 8 MiB worker stacks;
stop has execution capacity independent of ordinary business requests.

Login autostart initializes to enabled with `--background`. A background launch
opens existing node state without creating a window. The setting can be disabled
from the technical surface and subsequent launches respect the saved OS setting.
The initialization marker and node state live together; no identity is created
merely by launching an empty installation.

Linux writes its user entry under `$XDG_CONFIG_HOME/autostart`, with the normal
`~/.config` fallback. Executable paths and arguments use desktop-entry escaping,
including spaces. An external `Hidden=true` setting is reflected as disabled.

Windows registers only this application's values under the current user's
`Run` key. The executable and every argument use Windows command-line quoting,
including paths containing spaces. Registration respects the documented
[260-character Run value limit](https://learn.microsoft.com/windows/win32/setupapi/run-and-runonce-registry-keys);
the [process creation rules](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
explain why executable paths and arguments must be quoted.

The default node directory is the application's platform data directory plus
`node`. `--data-dir PATH` selects an explicit directory. Linux headless operation
uses the separate [service entry](../linux/README.md), without GTK or WebKit.
Both hosts and the application CLI use the same directory lease.

Only the bundled `main` window has the declared host-command capabilities.
Navigation is limited to the application origin and its CSP forbids frames and
external scripts. This surface is not a container for remote service webpages.

An explicit `--verification-dir PATH` enables a private filesystem controller.
The actual WebView reads numbered requests through IPC and submits normal host
operations, then writes exclusive result receipts. The mode is off unless that
argument is supplied. The controller directory must be private; Linux checks its
mode. Platform verification also checks the actual process, OS window, settings
and independently saved files, rather than treating these receipts alone as
acceptance.

`Cargo.lock` here is the desktop delivery graph. The SDK workspace lock stays
unchanged. `scripts/build_desktop_host.py` verifies frozen source, both locks,
the toolchain image and artifact budget before compiling under the repository's
shared Cargo lease. Build-generated schemas and command permissions are recorded
separately. Do not run an ad-hoc Cargo build or copy `target/` for another case.

Linux builds require [Tauri's WebKitGTK 4.1 prerequisites](https://v2.tauri.app/start/prerequisites/).
The initial Linux graph uses the pinned Ubuntu 24.04 toolchain image; current
runtime results and their precise host/session limits are recorded separately
under `docs/evidence/platform-host-2026-10-29/`. A compiled shell or an autostart
registration alone does not establish window or login-trigger qualification.
