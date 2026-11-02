# Throwaway inner-Web prototype

Question: can an independently admitted browser node use the existing member
connection to load and interact with a node-reachable HTTP page inside a
separate-origin client frame, without giving the page the management node?

Run from the repository root:

```sh
rtk proxy python3 browser-probe/prototype-web/run.py
```

The command prints a local URL. Open it, click **连接节点**, then **打开网页**.
The demonstration hostname `node-only.invalid` is resolved only by the provider's
owned HTTP adapter. The browser does not fetch that origin directly.

For the reproducible walkthrough and screenshots:

```sh
rtk proxy python3 browser-probe/prototype-web/run.py --check
```

The driver verifies the already-qualified Browser SDK from its originating
M103 receipt and selected inputs. It builds the prototype provider and owned
relay once through the shared Cargo cache with repository profile/features,
then reuses hash-verified immutable pins. Run logs and screenshots live under
`/tmp/miopunch-inner-web-prototype-20261021/`. The pins live on the repository
filesystem in the shared `.artifacts/` store, because cross-filesystem hardlinks
to `/tmp` are not supported here. No production node directory is used.

This is a protocol and page-container experiment, not the product UI or a final
HTTP module. It uses plain HTTP on owned local facilities, one preauthorized
browser, one provider-approved target, small requests/responses, and an external
curl process for each HTTP exchange. Cookies stay in the provider's disposable
single-user jar; this is not a production multi-user cookie design.

Expected checks cover resources, navigation, POST/redirect, cookie-backed login,
target refusal, connection/target failure and management-window isolation.
They do not qualify WebSocket, HTTPS, arbitrary absolute URLs, CSP, service-worker
applications, multiple target origins, streaming, full service ACL, Tauri or
Android. The Service Worker does not intercept native WebSocket handshakes.

All UI files, harness and `src/bin/prototype_inner_web.rs` are disposable. The
validated observations will be retained with the Plane prototype task; no
production protocol or container choice follows solely from this experiment.

The 2026-10-21 automatic walkthrough passed all eight checks, including clean
provider/relay exit and removal of the disposable private state. The final run
reused both native and WASM pins without invoking Cargo. Findings and evidence
are recorded under MIOPUNCH-108 and `docs/evidence/inner-web-prototype-2026-10-21/`.
