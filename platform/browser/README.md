# Self-hosted browser entry

This static delivery loads the project's real Rust/WASM node in the browser.
The small page is a technical host, not the deferred product UI. It needs no
native node, server-side identity, API backend or JavaScript dependency install.

Extract the generated archive into a dedicated HTTPS origin. Serve `.wasm` as
`application/wasm`, JavaScript as `text/javascript`, and the web manifest as
`application/manifest+json`. `localhost` HTTP is suitable for a local check;
ordinary non-local HTTP lacks the secure-context capabilities this entry needs.
Keep the supplied `pkg/` directory and its snippet paths together.

Node identity and saved membership are scoped to the browser profile and origin,
and remain in IndexedDB. Another tab on that origin is rejected while the first
holds the SDK's Web Lock. Closing a tab releases ownership without authoring a
membership removal; reopening can recover the saved identity and authorization.
Moving to another origin or clearing browser storage does not preserve that
state. Use the existing explicit recovery/backup operations when moving it.

For a local check from the extracted directory:

```sh
python3 -m http.server 8080 --bind 127.0.0.1
```

Open `http://localhost:8080/`. The public entry is `window.miopunch`:

```javascript
await miopunch.ready;
await miopunch.open();                    // existing or unadmitted node
miopunch.status();                        // public identity and current state
await miopunch.close();                   // wait for actual SDK/lease close
```

`create(configuration)`, `join(configuration, invitation)` and
`joinGuest(configuration, invitation)` expose the corresponding normal SDK
entry points. Configuration defaults to an empty override, retaining the SDK's
defaults and saved choices. The active `miopunch.node` exposes the current
`BrowserNetwork` public API, including explicit recovery-card export/save
acknowledgement, admission and business operations. A new network still stops
at its recovery-save gate; the page does not acknowledge that action implicitly.
Type signatures are shipped as `pkg/compiled.d.ts`.

Configure the HTTP server to avoid caching `index.html`, `entry.js` and
`release.json` across deployments, and publish each complete version together.
Add `frame-ancestors 'none'` to the server's CSP header. The bundled CSP permits
the SDK's configurable contact resources and forbids frames and external scripts.
No service worker silently swaps the SDK while a node is active.

`release.json` identifies the original WASM build receipt, source package hash,
current selected input hashes and every shipped file. Packaging verifies and
reuses those immutable SDK pins; it invokes neither Cargo nor wasm-bindgen.
The web manifest supplies a standalone entry, but browser installation and a
particular public deployment address require their own acceptance. Publishing
this bundle does not authorize overwriting the historical demo or an existing
origin's stored node.
