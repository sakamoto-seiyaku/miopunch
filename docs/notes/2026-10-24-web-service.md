# Application Web service integration

MIOPUNCH-119 implements stories 53/54 under the current M107 non-UI scope.
The earlier inner-Web prototype remains historical exploration.

Engineering choices for this implementation:

- A dedicated Web ALPN uses the existing authenticated connection, admission
  gate, signed `Service::Web` declaration and single service ACL. An explicit
  provider configuration limits effects to canonical HTTP(S) origins. That
  target scope grants no identity or service permission.
- The selected provider performs HTTP(S), including certificate validation.
  Redirects return to the consumer; each following request receives a fresh
  provider authorization check. There is no ambient proxy or provider Cookie
  jar. Each content container owns its cookies and passes them for its own
  requests. Public additional PEM trust roots are explicit provider inputs;
  certificate and hostname verification are never disabled.
- Native hosts have bounded begin/result/cancel request handles so an in-app
  content session can cancel HTTP without blocking its control pipe. Completed
  responses retain one of sixteen handles until consumed. Browser operations
  have an eight-owner limit. The Web module queues only contact handshakes to
  respect the existing shared contact budget; established traffic remains
  independent. This does not retry failures or change shared backoff.
- WebSocket messages use persistent directional futures and bounded queues.
  Both consumer implementations observe peer closure independently of a full
  application-reader queue. Explicit close and normal close are distinguished
  from authorization loss and incomplete delivery.
- Wire records and active work are bounded. HTTP bodies stream over existing
  bounded records; one public response is at most 8 MiB and a request at most
  1 MiB. These are implementation limits, not promises of arbitrary-site
  compatibility. Long operations belong to separate service workers so normal
  policy ingress remains available.
- Platform content hosts are minimal technical integration, without formal
  product UI. The accepted Vue/Tauri/Rust product responsibilities remain.
  Linux WebKit, Browser/WASM and actual Windows container evidence must be
  distinguished. A standalone WebKit technical host does not establish delivery
  of the final Tauri shell or its lifecycle.
- Browser technical hosting uses separate HTTPS management and content origins
  within one site. This preserves Strict cookies in an embedded content frame
  while host-only cookies, IndexedDB, localStorage and node objects remain
  separated by origin. Every content session gets a different hostname. The
  `.m119.test` names and the test browser's loopback-only resolver rule are
  owned fixtures, not product deployment domains or business-target filters.
  Equivalent origin routing and TLS are a real browser-host requirement; no
  third-party-cookie setting is relaxed to make the sample work.
- Browser test flags are not an egress boundary. A retained Chromium startup
  failure logged background registration traffic despite those flags. Current
  local scenarios place all SDK peers, relay, gateway, protocol targets and
  content engines inside a fresh loopback-only network/PID namespace with no
  effective capabilities. The source records older traffic and exit unknowns;
  current containment does not retroactively qualify them or choose a product
  VPN, firewall rule or deployment network policy.
- Windows preparation includes a locally type-checked CSharp5/x64 WebView2
  content host, its private-pipe wrapper, and pinned SDK/CLI artifacts. No node
  object or management message handler enters that content host. The concrete
  action remains explicitly not ready: fixed runtime availability, the Windows
  gateway binding, genuine whole-scenario network containment and actual
  Windows host/lifecycle/isolation qualification are still required. Local
  metadata compilation is not a Windows or content-isolation result.
- Explicit WebSocket close uses one five-second protocol budget for both
  outgoing enqueue and handshake, followed by a one-second cancellation/join
  budget for that exact owner. Dropping the close future also cancels its
  owner. A failed handshake cannot become a normal-close success. This is the
  Web module's local engineering bound, not a universal five-second product
  requirement. The independent close observer retains a fixed 6.25-second
  monotonic bound; existing incoming-pressure/provider-disable bounds stay five
  seconds.
- Normal-close qualification requires the SDK's successful outcome, content
  close code 1000 with `wasClean`, and settled module/target owners. Ordinary
  helper failures retain partial/final stdout and stderr before rethrowing the
  original failure. Shared future waits use their original monotonic budgets
  before stopping an owned event loop; the underlying cause of earlier
  premature timed-wait observations remains unknown.

Acceptance uses the public NativeClient/BrowserNetwork Web operations and the
actual content container, with independently observed fresh owned targets.
The initial current CLI rejected `web_enable` as `invalid_command` after a real
HTTP target, distinct member admission, synchronization and PONG succeeded.
That failure is retained under `.artifacts/runs/m119-web-entry-red-1/`.

Local Linux and Browser evidence covers real container resources, navigation,
redirects, login/Cookie, HTTPS, bidirectional WebSocket, current ACL and guest
lifecycle, origin/storage/native IPC isolation and owned cleanup. The receipts
retain original failures and scope each result to its actual source/platform;
an earlier sparse gateway cleanup failure remains unexplained. Actual Windows
qualification remains an obligation of this ticket. Earlier
M118 withheld networking, Windows SSH, external relays and restricted credential
operations are not authorized by this integration work.
