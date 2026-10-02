# Browser manual admission and durable identity

MIOPUNCH-50 implements the browser slice of the [current specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
Its [ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/97efef8b-1611-42b8-93b3-4350f121f1ee)
is limited to manual approval. Browser precode, richer control-state integration
and later platform tickets retain their own scope.

## Public seams and shared mechanisms

The parent specification already confirms client/SDK and native-node behavior
as the test seams. `BrowserNetwork` in the existing browser crate is the browser
SDK. `network.html` is a thin test page. `browser_network_native` drives the
existing `NativeNetwork` public invitation, approval, delivery and admission
gate APIs through a local operator stdin interface. The HTTP page server serves
only static client HTML/JavaScript/WASM. It neither approves applicants nor
holds their keys or admission materials.

The browser compiles the same `src/management_auth.rs` verifier and
`src/admission_proof.rs` SCRAM implementation as native nodes via Rust `#[path]`
modules. There is no second membership policy or password mechanism. The proof
module's only platform adaptation is a browser-safe timeout; native still uses
Tokio's existing timeout. `BootstrapProof::is_live` lets the browser reclaim
closed qualifications. The WASM dependency tree has no SQLite, SQLx, ractor or
native mio branch.

Browser admission uses the native manual protocol and its exact wire binding:
trusted root + canonical Create hash + contact identity, stable request ID and
TLS-authenticated applicant identity. The browser verifies complete original
signed management records, the canonical genesis, the provider's administrator
role and its own still-valid authorization before saving them. It acknowledges
`AUTH` only at completion's second-material step, acknowledges `SAVE` only after
the secret transaction commits, and consumes `DONE`. Local completion requires
the two committed materials, independently of the sender receipt or global
convergence. Dropping a pending delivery closes its connection, retaining the
durable request and saved authorization for a retry.

The endpoint installs the fork's `admission_gate`, which disables 0-RTT and
holds candidates/data release until qualification. Restricted manual requests
remain restricted. The controlled member ALPN requires the same native
SCRAM-SHA-256-PLUS proof bound to the current TLS exporter, verified membership,
and the native stage agreement. It explicitly returns `AcceptData` with no
custom carriers. No early data or raw unqualified endpoint handle is exposed.
The echo protocol is a controlled test exchange, not a service ACL decision.

## Recorded platform choices

These are reversible implementation choices for this slice, rather than new
membership rules:

- One origin owns one network identity. A page holds an exclusive Web Lock for
  its node lifetime. A second page reports that the identity is active, rather
  than creating a competing endpoint or writing another request. Closing or
  reclaiming the owner releases the lock; another page can restore the same
  identity. There is no claim that all pages concurrently operate the node.
- IndexedDB stores an entire versioned private snapshot: key, invitation,
  stable request, signed records, authorization, credential generation, secret
  and resource choices. Each replacement compares the previous snapshot and
  writes within one `readwrite` transaction using strict durability. Only
  `oncomplete` acknowledges the commit. Authorization and secret remain separate
  commits. This replaces the diagnostic echo's localStorage-only storage for
  actual network participation; the original echo probe remains separate.
- Startup replays the saved signed records and validates their authorization.
  Corrupt material fails closed; it is not silently regenerated. Actual material
  clearing is detected before later network use or commit. After a cleared
  profile is reopened, a new identity starts AwaitingApproval and needs explicit
  admission. No page event authors Leave/Remove. This slice does not add a backup
  UI, arbitrary profile recovery, or a browser control-history store.
- Configuration is a persisted startup snapshot. `{}` uses saved settings, or
  the pinned iroh production relay table on a fresh profile. `relays` explicitly
  replaces the allowed list; `[]` supplies no facility. An omitted `disabled`
  preserves local disables; an explicit list changes that local choice. URL
  origin normalization prevents paths/query strings/DNS suffixes from bypassing
  a disable. The final relay actor access policy permits only selected data
  relay origins and installs no external lookup or signaling facility.
- Web Locks requires a secure page context (HTTPS, or localhost for controlled
  development). Public deployment uses browser-reachable HTTPS/WSS relay URLs
  and a trusted static client origin. No endpoint is silently enabled when
  permitted relays are missing or unavailable. Missing facilities fail promptly;
  startup registration and exchanges are bounded to 12 seconds. Configuration
  changes take effect on the next start, not as a claimed live update system.

`BrowserNetwork.start(configuration_json)` loads or initializes the node.
`request_admission(invitation_hex)` saves its binding and returns
AwaitingApproval or PendingMaterials. `complete_admission()` commits the secret.
`status()` reports the loaded local stage and material-save facts; `running`
means the local node has not been closed, not that any peer is currently usable.
`exchange(message)` checks the persistent snapshot and performs a fresh qualified
member exchange. `close()` disconnects and releases the lease without removing
membership. The client uses no UDP/WebRTC direct path or background-residency
promise, and this does not browser-qualify the native maintenance runtime.

## Reproduce and observed scope

Use Rust 1.98.1, `wasm32-unknown-unknown`, wasm-bindgen CLI 0.2.129,
the locked harness dependencies (Playwright Core 1.63.0) and Chromium. The actual
final browser executable was Chrome for Testing 153.0.8010.12. If necessary:

```sh
rtk proxy npm ci --prefix browser-probe/harness
rtk proxy node browser-probe/harness/node_modules/playwright-core/cli.js install chromium
rtk proxy python -B scripts/check_browser_network.py --output /tmp/mio50-check --checks
```

The runner accepts `--wasm-bindgen PATH`, `PLAYWRIGHT_CHROMIUM_EXECUTABLE`, and
`--case NAME` for one incremental scenario. On this machine it locates the
existing `/tmp/miopunch-wasm-tools/bin/wasm-bindgen`. Output must be a new directory.
`--skip-build` reuses the existing build when intentionally checking a harness
condition. All owned relays, node directories and browser profiles are isolated;
the runner closes its fixtures and removes their private material.

The first genuine browser RED was the old WASM's missing BrowserNetwork export.
The first GREEN preceded the priority freeze/reopen check. The final consolidated
suite then ran eleven real cases over HTTPS/WSS: first admission/refresh, page
freeze/reopen, physical Chromium process restart on a persistent profile,
competing-page exclusion/takeover, authorization-only interrupted delivery,
material clearing/readmission, actual secret-commit loss, empty allowed relays,
normalized relay disable, a real unavailable permitted relay, and TLS/WebSocket
access. Native Add counts remained unchanged across retries, pauses and reopens;
only explicitly approved new identities added a record. Both final fixtures
exited 0. Actual request monitoring across all tested contexts observed only
the static origin and selected/explicitly failed test resources.

The self-hosted TLS fixture runs on loopback with a generated certificate.
Native explicitly trusts that test CA; the isolated Chromium fixture permits
that test certificate. End-to-end iroh QUIC/TLS and signatures are real. This
qualifies the HTTPS/WSS mechanism and client semantics, not public-origin
availability, production certificate issuance, another browser/OS, arbitrary
browser process crashes, or a security audit. The bundled public table remains
best effort; no production relay availability claim is made.

The final affected native regressions report 10 proof, 18 gate, 14 manual
admission and 1 RFC vector passing entries. The manual count includes two
inert worker entries and one MQTT scenario which returns early without a
configured broker. Those entries are not fresh MQTT/browser acceptance.
Workspace typecheck, WASM build and formatting pass. The user's instruction
excludes unchanged slow NAT matrices. See the [evidence index](../evidence/mio50-browser/README.md)
for actual commands, exits, cases, failure classification and review results.
