# miopunch

miopunch is an experimental identity-based private network for personal and
small-group use. The repository now contains Rust native and browser/WASM node
implementations, dependency forks, runnable probes and validation evidence.
The [initial application scope](docs/notes/2026-10-03-shared-web-client-ui-direction.md#首版功能范围五个独立业务模块)
now comprises one-to-one clipboard synchronization, file transfer, TCP/UDP port
forwarding, browser access to internal web services, and remote shell with
concurrent sessions retained through tmux.
Planned first-release platforms are Windows, Linux (including headless nodes),
Android, and browser/PWA clients, with services matched to platform capabilities.
These application modules now have a scoped [Linux/browser demo](demo/README.md);
product-wide platform, lifecycle and integration acceptance remain incomplete.

The accepted [UI stack](docs/adr/0018-shared-vue-ui-with-tauri-shells.md) uses
Vue 3, TypeScript and Vite for shared interfaces, Tauri 2 for installed
application shells, and Rust for node and application logic. Platform adapters
provide system integration; browser/PWA clients remain independent WASM nodes.
The explicitly authorized demo exercises the shared interface. Minimal
[Tauri desktop hosts](platform/desktop/README.md), a
[Linux headless host](platform/linux/README.md), and a
[self-hosted browser entry](platform/browser/README.md) now have bounded
[platform qualification and fixed-artifact records](docs/evidence/platform-host-2026-10-29/linux-login-qualification.json).
Linux login autostart is qualified in an isolated LightDM/PAM/LXSession session.
Windows login autostart, Android foreground/reboot, clipboard and WSL acceptance
remain incomplete; the complete shared client UI remains to be implemented.
The browser client will have a default access URL and support self-hosting.

The accepted client behavior, platform requirements and validation scope are
consolidated in [MIOPUNCH-107: first-release client PRD](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6159ec62-a03e-4eb7-9b95-4ddc19ce7a02).

Start with [CONTEXT.md](CONTEXT.md), the [product direction draft](docs/notes/2026-09-27-product-mainline-draft.md), and the [scenario walkthrough](docs/notes/2026-09-28-network-scenario-walkthrough.md). These documents distinguish confirmed behavior from open design questions. Research reports in `docs/notes/` are evidence, not approved implementation plans.

The [network foundation specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)
collects the accepted decisions, module responsibilities, behavioral acceptance
criteria, and technical prerequisites still requiring validation. Use it with
the current source and scoped evidence; the September design documents retain
some historical implementation-stage descriptions.

## Current engineering map

- [Dependency fork changes and compatibility](docs/notes/2026-10-12-iroh-p2panda-fork-change-audit.md): original upstream baselines, exact change counts, concentrated modifications and interface/behavior risks.
- [Architecture, operation flows and measured performance](docs/notes/2026-10-12-current-architecture-flow-and-performance-map.md): current native/browser responsibilities, admission and connection lifecycles, synchronization, measurements and open validation boundaries.
- [Machine-readable fork inventory](docs/evidence/fork-change-audit-2026-10-12/README.md): per-file changes and counting method.
- [Application Web service integration](docs/notes/2026-10-24-web-service.md): provider-bound HTTP/WebSocket and content-host integration, platform evidence, and current Windows qualification boundaries.
- Compatibility feasibility, assessment only: [iroh and its transport dependencies](docs/notes/2026-10-12-iroh-upstream-interface-compatibility-feasibility.md), and [p2panda with current demo dependencies](docs/notes/2026-10-12-p2panda-and-demo-compatibility-feasibility.md). Proposed changes have not been implemented or regression-tested.

These are snapshots for understanding and maintaining the current code, not
new product decisions or instructions to start a refactor.

The former Go/Android prototype, OpenSpec artifacts, lab, scripts, and local
agent prompts are retained in [the legacy archive](archive/legacy-go-openspec-2026-09-28/README.md).
The archived implementation and test commands are not automatically the new
baseline. Whether to reuse, adapt, or replace the old validation tooling is
still undecided. Existing local build output in `dist/` is retained but not
tracked.

Work items and decision tickets live in the Plane `MIOPUNCH` project. See
[the agent tracker guide](docs/agents/issue-tracker.md).
