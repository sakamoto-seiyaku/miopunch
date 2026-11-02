# Application Web service integration

[MIOPUNCH-119](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/26c5a8ef-787f-4983-8ddf-f9ba53c102c1)
implements stories 53/54 under the current M107 non-UI scope.
The earlier inner-Web prototype remains historical exploration.

Status (2026-10-29): the M119 non-UI acceptance criteria are satisfied.
The [final evidence mapping](../evidence/m119-windows-host-2026-10-25/m119-acceptance-20261029.json)
combines both Windows/Linux roles, current Native and browser regressions,
Windows service-policy/guest checks, browser natural expiry and owner cleanup.
The dated incomplete results below remain as historical evidence. M121 platform
lifecycle and formal product UI are outside this acceptance.

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
  Linux/browser-only local scenarios place all SDK peers, relay, gateway,
  protocol targets and content engines inside a fresh loopback-only network/PID
  namespace with no effective capabilities. The source records older traffic
  and exit unknowns; current containment does not retroactively qualify them
  or choose a product
  VPN, firewall rule or deployment network policy.
- Windows uses a CSharp5/x64/net48 WebView2 technical host and original private
  pipes. A real ordinary-user desktop run now qualifies local HTTPS-page
  loading, evaluation and host/input/engine/gateway shutdown, as detailed below.
  The separate headless SDK/provider/gateway case and an actual provider-bound
  WebView2 page/resource entry now pass in that ordinary context. A later
  instrumented case also records content/login/WebSocket and positive-control
  storage/IPC no-effect observations. The original asynchronous close callback
  and attached-client closure failures remain retained. The later ONE902 and
  marker-corrected policy/guest cases qualify clean transport lifecycle for
  their frozen scopes. No node object or
  management message handler is registered by the host; a configuration flag
  alone does not prove content isolation.
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
an earlier sparse gateway cleanup failure remains unexplained. The Windows
headless SDK/public-Web/gateway case now passes separately. The instrumented
Windows content cases now include a qualified callback-clean closure case and
current Web policy plus scoped-guest explicit-end cases. Windows/Linux role
exchange, remaining platform coverage and formal review remain obligations.
Earlier
M118 withheld networking, external relays and restricted credential operations
remain outside this integration work. Windows SSH is used for bounded fixture
deployment, control and evidence; business traffic must use the actual SDK path.

## Windows runtime observations, 2026-10-25

The [curated qualification record](../evidence/m119-windows-host-2026-10-25/qualification.json)
retains source/artifact pins, result boundaries and hashes of the original raw
receipts. [Runtime pins](../evidence/m119-windows-host-2026-10-25/runtime-pins.json)
fix the app-local x64 CPython 3.13.17 and twelve compatible Windows/pure wheels,
including aiohttp 3.13.3, cryptography 46.0.5 and cffi 2.0.0. These are technical
fixture dependencies, not a new product runtime choice. Actual imports passed
with site initialization disabled. No system Python PATH or registry setup was
needed.

The existing WebView2 155.0.4283.62 runtime was reused. The executable SHA256 was
checked before and after the run, and the actual BrowserVersionString and fresh
UserDataFolder matched the host options. This is evidence for an existing
versioned Evergreen directory, not a complete immutable Fixed Version package.
The host compiled against pinned WebView2 SDK 1.0.2849.39 and net48 references;
the reference assemblies are not deployment frameworks.

| Actual entry | Observed result | Qualification |
| --- | --- | --- |
| SSH-created host, final source graph | Own HWND/PID/UI-thread checks passed; `EnsureCoreWebView2Async` failed with `80070578` before readiness | No page or engine-settlement pass |
| Same final host/controller/runtime in the same account's ordinary desktop session | Session 1, interactive, no enabled Administrator role; actual HTTPS page and private evaluation passed | Technical host plumbing only |
| SSH-created Windows SDK11 public provider and consumer entry | Provider creation/readiness and normal exit passed; consumer `join` exited with OS error 5 before readiness/destination publication | Original failed case retained |
| Same cached SDK11/controller/runtime in an ordinary desktop context | Distinct member admission, exact async HTTP, provider-bound gateway HTTP/WS and pending cancellation passed | Headless SDK business |
| Actual SDK11/provider/gateway through WebView2 in an ordinary desktop context | Real page script/fetch, applied CSS and decoded image passed; original host/input/engine/SDK/gateway/target owners settled | Integrated page/resource entry; remaining content, isolation, ACL/guest and cross-platform cases open |
| Later content/isolation case in the same ordinary desktop context | SDK11 provider public `create` exited with OS error 5 before readiness; no consumer or renderer was created | Whole case failed; content and isolation assertions unreached |
| Instrumented SDK11 in an ordinary desktop context | One create, pre-admission join and reopen of the same pending node succeeded; all original processes/readers settled | Unlocalized within this case; original initialization failures remain unresolved |
| Instrumented SDK11 full content case in an ordinary desktop context | Login/Cookie/303/navigation, native WebSocket, gateway denials and positive-control isolation observations passed; outer stderr recorded unhandled callback error 10054 | Functional coverage qualified; callback/transport lifecycle unqualified, original `PASS` label retained |
| Same content operation with owned-loop observation and repeated-close handling | Business assertions passed and callback stderr was empty; main gateway HTTPS server still recorded three attached clients after loop close | Attached-client-zero oracle failed; original `PASS` label retained, no clean transport lifecycle acceptance |
| Same content operation after the registered-server TLS closure fix, explicitly authorized ONE902 run | Business assertions passed, pending close took 0.646s, five loops closed and six servers had zero attached clients/listeners before loop close and at final cleanup | Scoped Windows technical-host pass; original failures retained, full M119 remains incomplete |
| Explicitly authorized Windows policy case on the same retained SDK | Provider create failed at `MoveFileExW_no_replace` with error 5 after preparation/SQLite close; original process and Task terminal, no consumer or renderer created | Policy/ACL assertions not reached; conditional guest case not run |
| Marker-corrected policy followed by conditional scoped-Web guest/end, 2026-10-27 | Both complete business and independent directory-observer qualifications passed; 50 fixed files verified, all original owners/loops/servers settled | Current Windows Web policy and scoped guest explicit end qualified; full M119 remains incomplete |

[Microsoft documents `ERROR_INVALID_WINDOW_HANDLE`](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.winforms.webview2.ensurecorewebview2async)
as an invalid host-parent HWND during WebView2 initialization. The controlled
desktop comparison demonstrates a context-dependent failure in this harness;
it does not establish a universal restriction for every Session 0 application.
Explicit `CreateControl` and a valid own HWND were insufficient in the SSH case.
The host's shutdown wait was also moved inside the live UI message loop instead
of waiting for a UI-dispatched engine-exit event after `Application.Run` ended.
The original deadlines remain: initialization 10s, evaluation 5s, aggregate
input/initialization/engine settlement 5s, and parent readiness observation 15s.

The successful desktop case loaded a fixed harmless page from the owned
loopback HTTPS gateway. Its callback supplied that fixture directly; no SDK or
remote provider participated. Private close closed the original stdin, joined
the input reader, observed the browser-process exit event and joined the
original host process with exit 0 and no live readers. The gateway thread,
event loop and executor joined. The early bad-pin case reported the exact
hash-guard failure without creating a content profile. All three original SSH
failures and their false/unqualified engine-exit observations remain retained.

The desktop comparison ran automatically through one same-account,
least-privilege, demand-only temporary task using
[the documented interactive-token context](https://learn.microsoft.com/en-us/windows/win32/taskschd/security-contexts-for-running-tasks).
It had no triggers or password, and the child verified its actual SID,
interactive session and non-administrator context before running the frozen
fixture. The original task instance, exact registration and completed child
were checked before deleting that one registration and verifying its absence.
The observer's `8004130b` after the instance ended remains recorded; completion
was qualified with the exact owned task's zero-instance state and original
child receipt, rather than the observer exception or last task result alone.
This is test coordination, not implementation of product login/autostart.

Generic `chrome.webview` was present in the actual page. Its presence is not
evidence of a node-management bridge, and the host's declared lack of management
handlers is not an actual IPC-isolation test. The later instrumented case below
records separate positive storage controls and actual IPC/file/origin attempts
with no effect. The later marker-corrected policy/guest cases qualify effect-time
ACL and scoped guest explicit end; they do not cover every guest end condition.
Engine-background
destinations were not independently observed in this ordinary-desktop run;
it makes no whole-host zero-egress or namespace-containment claim.

The SDK comparison reused the exact Windows SDK11 PE, app-local runtime and
controller without recompiling Rust or copying another SDK. A second fresh,
same-account, least-privilege demand-only task verified Session 1, an interactive
context and no enabled Administrator role before launching the SDK. Public
`join` completed for a distinct consumer identity. Disabled Web service caused
no target effect; asynchronous HTTP returned the exact independently observed
body. The real provider-bound gateway completed HTTP and bidirectional
WebSocket text/binary exchange, with close code 1000 and the SDK reporting
`closed: true, failed: false`. A request that had actually reached the target
was cancelled and joined, its stale handle was rejected, and remaining request,
socket and flow counts were zero.

Both original SDK process handles exited with code 0 and no live readers. The
gateway thread, loop and executor joined, and the target reader and WebSocket
owners settled. Cleanup, unsettled-owner and source-drift lists were empty.
The original task instance completed in 17.33s; that exact registration was
deleted and its absence verified. The sixteen original raw files and their
hashes are retained. This headless SDK/gateway case is separate from the
earlier fixed-page WebView2 host case and the later integrated entry below.

A subsequent fresh ordinary-context case joined the actual paths: SDK11 provider
and distinct consumer, real SDK gateway callback, and actual WebView2 navigation.
The SDK returned the page, stylesheet, image, script and script-initiated fetch
from the owned HTTPS target. The real page observed the expected fetch result,
computed CSS `rgb(12, 34, 56)` and a decoded image of width 2. Independent target
observation and the SDK response records accompanied those page assertions.
The original host HWND and UI thread matched before and after initialization.
Private close then joined the same stdin/input reader, observed the actual
engine-exit event and joined the host with exit 0 and no live readers. Both SDK
processes, gateway thread/loop/executor and target reader/socket owners settled;
cleanup, unsettled and drift lists were empty, and the runtime hash was unchanged.
The exact temporary task completed in 25.14s and was removed. Nineteen original
raw files are retained. This qualifies the integrated page/resource entry;
it does not yet qualify the remaining content, isolation or ACL/guest cases.

The later content/isolation case failed before reaching those assertions. In
the same account's Session 1, with an interactive, non-administrator context and
verified unchanged pins, the SDK11 provider's public `create` exited with
`Io(Os { code: 5, kind: PermissionDenied })` before readiness. The original
provider handle exited with code 1 and no live readers; no consumer, content
renderer or management-control renderer was created. The target preconditions
and target cleanup passed, but the case retained its provider cleanup error and
whole `FAILED` result. The original Python process exited with code 1. The exact
temporary task was terminal and removed; that coordination success is separate
from the failed SDK entry. Partial collection retained thirteen existing raw
files and the explicit missing-file errors for nine files never created.

This ordinary-context `create` failure rules out an SSH-only explanation for
SDK OS error 5. Earlier ordinary-context join and resource-entry passes remain
valid evidence for their exact cases, but do not establish reliable Windows
initialization. The original SSH `join` failure also remains unclassified: its
inputs were readable, destination absent and failing operation unknown. The
shared create/join initialization, key/anchor/binding storage, SQLite lifecycle
and atomic directory publication now require expanded diagnosis before another
case attempt. The diagnostic source is prepared from the original frozen SDK11
archive. It adds tagged operation boundaries to three files covering shared
initialization, file publication and client startup, with static path kinds and
error codes. It records no private values and is intended to preserve the original
operations and errors. Replay also exposed an archive-completeness gap: the native
snapshot retained a workspace-member manifest but omitted its library entrypoint.
Cargo exited during workspace parsing without emitting a compiler artifact.
A matching historical Browser archive had all 535 original native members byte
identical, and supplied the exact 32 omitted files. The reconstructed 567-file
workspace kept the original manifests and lock, with only the three diagnostic
source changes. Its Windows diagnostic PE compiled successfully and was pinned
at SHA256 `70aba7ef847b4ca4409c28d643ba6ac1a5bf32ecbc28b6ed48c70529cd1c1f40`
(187,218,409 bytes). Actual counts were 519 Fresh and 9 rebuilt: two own targets
and seven unexpected dependencies. All 528 artifact identity records and all 485
depfile hashes matched the original graph; the prior cache-dirty trigger remains
unknown because its trace was not retained. No extra compilation was used to
manufacture cache-hit evidence.

The outer driver failed after pinning because its JSON report received a Path
object. That failure and its stale fields remain preserved; a separate receipt
qualifies the actual successful compilation, immutable pin and exact compiler
container retirement from the raw evidence. This build preparation does not
replace or invalidate the retained runtime results. The diagnostic binary has
not yet localized the failing SDK operation or cause.

The subsequent ordinary-context diagnostic case reached readiness once through
public create, join and open. The creator stayed online while the independent
joiner reached `AwaitingApproval`; that joiner then stopped and reopened with the
same saved identity. No admission request, redemption or JoinComplete was asserted.
All three original SDK handles exited with code 0 and no live readers; cleanup,
unsettled and source-drift lists were empty. The original Python process exited
with code 0, and the exact temporary task completed in 8.91s and was removed.
Nineteen raw files (53,258 bytes) are retained. All 270 tagged rows had no error
or raw OS code, including SQLite close, publication, resource saving and startup
boundaries. The result is `UNLOCALIZED_WITHIN_THIS_CASE`: neither original OS
error 5 was reproduced or localized, and instrumentation may affect timing.
Reliable initialization remains unproved.

The next full content case reused that diagnostic PE and reached the previously
unreached assertions. Real form POST, target-received Cookie, authenticated read,
303 redirect and navigation passed. Native WebSocket supplied an unsolicited
greeting, exact text and 37 binary bytes, carried the login Cookie, and closed
with code 1000 while the SDK reported `closed: true, failed: false`. Seven real
gateway denials returned their expected 403/404 results and left the independent
target count unchanged at 7. A second actual renderer used a different profile
and origin to seed and reread localStorage, Cookie and IndexedDB. Content IPC,
file and other-owned-origin attempts left those controls, the owned file,
provider configuration and independent targets unchanged. Generic WebView was
present and postMessage did not throw; neither was used as a permission oracle.
This coverage applies to the current typed host, whose native management handler
declaration is false.

The target observed the pending request before gateway close, which completed in
0.353s. Both host protocols recorded input join, actual engine-exit events,
process exit 0 and no live readers; SDK, gateway and target summaries also
recorded settlement and no source drift. The exact temporary task completed in
38.62s and was removed. All 22 raw files (115,270 bytes) are retained.

The original `PASS` result is preserved, but a clean lifecycle pass is not
accepted: the outer Python stderr contains an unhandled
`ConnectionResetError: [WinError 10054]` in
`_ProactorBasePipeTransport._call_connection_lost` (441 bytes). The current
fixture catches exceptions escaping its loop runners, while internal callback
errors are logged without entering its error lists. Thread/loop/executor and
WebSocket counts alone therefore do not establish callback-clean HTTP/TLS
transport closure. The exact loop, connection and phase remain unknown; the
reset has not been classified as harmless. Own-loop error/phase/transport
observation now covers the shared gateways, targets and temporary direct-client
loops while retaining default reporting and the original business assertions.

A separate local regression exercised the real gateway and target constructors,
normal close, and a replay of the recorded error context through each owned
loop's exception handler. All four pre-fix cases failed on repeated close:
cleanup rescheduled work on a closed loop, lost the first outcome and, for the
gateway, emitted an unawaited-coroutine warning. The fixed sources passed the
same four cases: repeated close reused the first receipt or error, submitted
no new work and emitted no such warning; original loops, threads, executors,
listeners and attached clients settled. Both replayed exceptions remained
visible through the default reporter. This used Linux Python 3.13.11 and the
same frozen dependencies for both versions; it proves the repeated-close fix
and error propagation, not reproduction or repair of Windows Proactor I/O.
The frozen next packet contains 15 files (180,004 bytes) and reuses the deployed
SDK, host and runtime. No Windows callback fix or reliable startup claim follows
from the local result.

The separately approved deployment and single observed content run then
completed in 44.65s, with the exact temporary registration removed. All 22 raw
files (181,382 bytes) are retained. The original Python process and all four
SDK/host processes exited with no live readers; both hosts reported input join
and actual engine-exit events. Business assertions passed again. The five
observed Proactor loops reported no callback failure or overflow, and outer
callback stderr was empty. These observations do not resolve the prior 10054
failure.

The stronger closure oracle failed: the main gateway's original HTTPS server
recorded three attached clients throughout shutdown and after its event loop
closed, despite having no listening socket. Its early-close receipt is in the
event log; the final cleanup list skips that already-closed gateway. The other
gateway, two targets and direct-client loop recorded no remaining attached
clients. The pending request was independently observed, and gateway close took
0.332s against the unchanged five-second bound. The recorded attached-client
membership does not establish a live OS socket or process, and the individual
transport states were not captured. The original `PASS` label and all raw
observations remain unchanged; the attached-client-zero requirement prevents
clean lifecycle acceptance. The single authorized run has not been repeated.

A local real-TLS reproduction then isolated a shared fixture cleanup defect.
For both the gateway and target, one verified TLS client kept open during
server close left one attached transport after loop close. Its recorded socket
was still unreleased with a nonnegative file descriptor; TLS state was
`SHUTDOWN`. Both client-closed-first controls passed. This reproduces the local
closure predicate without requiring a pending request; it does not identify
the states of the three earlier Windows transports.

The shared helper now closes each explicitly registered server's clients and
awaits `Server.wait_closed()` on its original loop before that loop stops.
Both gateway and target shutdown call it after HTTP-runner cleanup and enforce
zero attached clients and listeners. The same four TLS cases then passed in
about 0.263s with the client still open in the primary cases. The affected four
callback/repeated-close cases also passed on the same fixed source graph,
preserving the first outcome and default reporting of both replayed errors.
No assertion, timeout, default event-loop policy or client/host shutdown order
was relaxed. This first qualification used local Python 3.13.11; the later
Windows ONE902 result is recorded below. An older fixture runtime
without the public client-close API explicitly fails with active clients;
this does not decide a product runtime version.

## Independent local closure checks, 2026-10-25

The [local qualification record](../evidence/m119-local-web-close-2026-10-25/qualification.json)
retains two real ordinary entries and a separate Browser pending-close failure.
They reuse the original Native SDK11 and WASM5 artifacts, archived controllers
and the locally qualified TLS-server closure helper. No Rust build, new binary
pin or native-library copy was needed. Each case ran inside its own fresh
loopback-only network/PID namespace; namespace checks preceded the actors.
The GTK/WebKit host used existing system Python 3.11, while the controller used
the retained Python 3.13.11 environment.

| Actual entry | Observed result | Qualification |
| --- | --- | --- |
| Independent WASM5 with Chromium content container | Distinct identity, admission, signed service sync, real resources, login/Cookie/303/navigation and bidirectional WebSocket passed in 26.33s | Ordinary close passed; this entry contains no pending-close case |
| Native SDK11 with Linux WebKit content host | Real resources, login/navigation, WebSocket, seven gateway denials with unchanged target count and pending gateway close passed in 14.17s | Pending close took 0.285s before host termination observation; HTTPS attached clients reached zero before loop close |
| Same independent WASM5 with a real unreleased pending request | Gateway close took 14.825s against the unchanged five-second bound while the node, browser and content frame remained alive | Original assertion failed; immediate SDK-operation/provider-flow and renderer-failure assertions were not reached |
| Current WASM with per-request cancellation, first real pending-close regression | Gateway close took 0.282s; the node/frame stayed alive, immediate SDK owners settled and same-node target-observed follow-up passed | Whole case later failed in a second TLS target constructor; remaining result/handle/stop checks were not reached |
| Same current WASM after the fixture correction | Full pending-close regression passed in 53.01s; gateway close took 0.263s with the node/frame alive and target unreleased | Exact-owner negatives, provider-origin denial, eight retained handles, stop cleanup and final active request/socket/flow zero all passed |
| Same current WASM, affected ordinary Browser scenario | Real resources, login/Cookie/303/navigation, WebSocket and seeded management-storage isolation passed in 25.07s | Same pins reused; all recorded owners/readers, four observed loops and four registered servers settled |

Both ordinary cases recorded zero attached clients, listeners, callback failures
and observation overflow after shutdown. All original owned processes and
readers were terminal, private state was removed and source-drift lists were
empty. Browser stderr retained isolated DBus/Fontconfig warnings and its own
loopback DevTools endpoint; Native stderr retained WebKit/a11y warnings. The
Native fixture also retains expected negative authentication/display probes
and its owned Xvfb termination result. These are scoped observations, not a
claim that every child exit code was zero.

The Browser ordinary case seeded and reread actual management localStorage,
Cookie and IndexedDB controls, which content attempts left unchanged. Native
file/provider no-effect checks and actual isolation attempts passed, but its
management JavaScript storage-absence checks lacked separately seeded positive
controls. That case does not complete combined native management-bridge
isolation acceptance.

In the failed Browser pending case, the target had observed the request and
remained unreleased throughout gateway close. The gateway shutdown future
finished in 2.254s, with TLS clients falling from three to zero. The original
WASM `request_web` callback returned a public error about 12.571s later,
immediately before gateway close returned. The exact cause of that error is
unknown. Source and event timing indicate that the remaining executor join
waited for the Browser RPC; no live-stack probe was taken. Final cleanup
settled all owners, but cannot establish the immediate cancellation predicates
skipped by the earlier bound assertion. The original failed result is retained.

The frozen Browser interface exposes a request Promise and whole-endpoint
cancellation, without Native's per-request begin/result/cancel interface. Its
gateway callback therefore had no registered request handle to cancel during
close. The current Web interface and adapter now compile and the first real
pending-close core passes, as scoped below. Full regression acceptance remains
open. The five-second assertion and client-alive shutdown order are unchanged.

Retaining these additional source snapshots and failures required an explicit
local raw-evidence allowance change from 16 MiB to 20 MiB. Counting uses the same
two receipt parents and includes the retained source copies; there is no
exclusion or baseline reset. The measured total after the failed pending case
was 17,543,435 bytes; including the unchanged future Windows reserve gives
19,823,519 bytes. A proposed source-copy deduplication stopped when the required
host process-check report failed on `/proc/1/ns/pid`; no cleanup or alternate
access followed, and zero freed bytes are claimed. Artifact-store and remote
budgets were unchanged.

The subsequent Web source and assertion review found no blocking issue. A
single current-source WASM graph compiled in 17.38s with 295 Fresh dependencies
and one rebuilt Browser target. Its frozen closure contains 229 source files
and thirteen metadata inputs
with the original workspace lock. Native terminal implementation and tests are
excluded; existing Browser/shared terminal modules are transparently included,
unchanged by this Web fix, without a terminal-behavior qualification claim. The full source
archive, build logs and two entry receipts have an explicitly revised local raw
allowance of 24 MiB, increased from 20 MiB with the same counting and receipt
parents. All 370 existing store objects are retained. The existing 33 GiB store
ceiling is explicitly extended to this one WASM graph, with new pins capped at
108,003,328 bytes and the full M120 reserve preserved; the default 32 GiB policy
and cumulative baseline/cap stay unchanged. The M119 cohort allowance changes
from 256 MiB to 320 MiB to include this graph. Actual new store growth was
101,760,095 logical bytes and 101,769,216 allocated bytes, creating five new
objects; all 370 previous objects remain. Nine raw/package pins were checked
against independent store objects. No Native or Windows graph was built.

The [current WASM qualification](../evidence/m119-local-web-close-2026-10-25/current-wasm-qualification.json)
retains the first corrected entry and its remaining failure. The same pending
operation closed in 0.282s while its target stayed unreleased and node/frame
stayed alive. Immediate Browser-operation/provider-flow settlement, renderer
failure and management controls passed. The same node then made an exact,
independently observed request. Cancelling one of two pending handles left the
other pending; stale/unknown IDs and a live WebSocket ID were rejected without
affecting that socket. A completed response was consumed once.

That first corrected case failed after 29.56s: creating a second TLS target reused
the first target's exclusive `target-key.pem` path. That constructor had already
created a loop but had not registered the target. Its loop emitted only a
`created` observation, with no server or thread started and no explicit close
observed. Registered processes/readers and the other three observed loops
settled, but their summaries do not prove cleanup of this unregistered loop.
All 35 raw files (302,471 bytes), original source snapshots and failed result
remain retained. Provider-origin denial, eight retained handles, `stop_web` and
final follow-up checks were not reached; the ordinary Browser regression was
not launched.

The reviewed fixture correction gives each target distinct TLS filenames and
prepares certificates before allocating a loop. The origin-denial check now
uses a fresh HTTP target with an independently successful direct response,
removing certificate trust or target unavailability as alternative causes of
no effect. A real two-TLS-target and exclusive-file constructor check ran before
the same pending and ordinary regressions, without a new Rust build.
The evidence allowance for this same runtime family explicitly increases from
1 MiB to 2 MiB to retain the first failure and follow-up evidence; the global
24 MiB raw allowance and all store/baseline/reserve limits remain unchanged.
The [fixed runtime qualification](../evidence/m119-local-web-close-2026-10-25/fixed-runtime-qualification.json)
records that subsequent sequence passing. Constructor checks took 0.566s:
both TLS targets remained usable after the actual exclusive-file failure, with
exactly two created loops and two closed final summaries. The full pending case
then passed, including all previously unreached controls, and the ordinary
Browser scenario passed on the same WASM pins. Each Browser case had four
observed loops matching four closed summaries and four registered servers with
zero attached clients/listeners, callback failures and observation overflow.
All original registered processes/readers settled, private state was removed
and source-drift lists were empty. The provider's cumulative failed count of
five in the pending case reflects expected denial/cancellation cases; its
active-flow/request/socket counts were zero. Browser stderr retains isolated
DBus, own-loopback DevTools and Fontconfig messages, without an asyncio callback
exception. No extra compilation or pin was needed for these three cases.
Both original failures remain unchanged in the record.

The [current Web ACL/guest qualification](../evidence/m119-local-web-close-2026-10-25/guest-qualification.json)
then passed the complete existing issuer/consumer scenario in 34.28s on the
same WASM and Native SDK11 pins. A Browser administrator issued a Web guest to
an independent Native identity: pending access had no target effect, activation
enabled exact HTTP and a real WebSocket, current ACL denial closed the socket,
restored access retained the original deadline, and issuer revocation closed
and denied access. A Native issuer then admitted an independent Browser guest;
its own page reload preserved identity and the original deadline, and local
guest exit delivered the end record and closed the actual socket. Subsequent
requests were denied without target effect. This is the known Web persistence
scenario, with its original assertions and waits retained.

All 37 raw files (237,413 bytes) were hash-checked. Two observed/created loops
matched two closed summaries; three registered servers had zero attached
clients/listeners, callback failures and overflow. Seven original owner
histories, readers and the observed Browser subtree settled; private state was
removed and source drift was empty. The outer helper's successful retirement
and subsequent successful cleanup record were both checked. Browser stderr
retains isolated DBus and own-loopback DevTools messages. Qualification-script
lookup errors remain recorded; they were corrected against the same raw data
without rerunning the case. No new build, pin, native library or Core source
copy was needed. The same runtime family's evidence allowance explicitly
increased from 2 MiB to 3 MiB; the global raw allowance stays 24 MiB.

The separate [Browser guest natural-expiry qualification](../evidence/m119-local-web-close-2026-10-25/natural-expiry-qualification.json)
passed in 66.63s on the same pins. Before the original 45-second deadline, one
real pending HTTP request and one live WebSocket accounted for exactly two
client operations and two provider flows. Both closed within 0.43s of the
first observed current expiry, within the unchanged ten-second bound. The HTTP
error was consumed 3.44s after the request began, before the independent
fifteen-second read timeout with the required one-second margin. The same guest
identity and deadline remained, with `GuestExpired` and `ended=false`; the node,
provider, browser and unreleased target stayed alive through the assertions.

New HTTP and WebSocket attempts failed. Unchanged source assertions checked
request, connection and message counters after each attempt and at the final
marker; numeric counter snapshots were not separately persisted. The final
predeadline sample was only about 2.4ms before expiry, so the source's scheduling
threshold does not establish a 0.75s margin. No invalid wall-clock window was
observed, and the single hundred-second total budget was retained.

Independent review checked all 36 raw files (265,755 bytes), 108 events, two
created/closed loops, three servers with zero attached clients/listeners, five
process owner histories and fourteen observed Browser descendant identities.
Readers and threads joined, callback failures/overflow were zero, private state
was removed and source drift was empty. The audit's initial budget-counting
error is preserved; correcting it to the existing raw-budget expression needed
no runtime retry. Retaining both guest cases explicitly increases their
combined increment allowance from 1 MiB to 1.5 MiB within the unchanged 24 MiB
global and 3 MiB family limits, preserving the Windows reserve. This case needed
no new build or pin.

After explicit user authorization, the same 15-file packet (182,932 bytes) was
deployed and its [single Windows TLS-closure case](../evidence/m119-windows-host-2026-10-25/tls-closure-qualification.json)
passed. The ordinary interactive session had no enabled Administrator role.
All 476 manifest files and the existing runtime were verified before execution;
the manifest remained unchanged afterward. No SDK, host or runtime was rebuilt
or recopied. The reviewed 24 MiB preflight used the accepted 375-object store
and 320 MiB cohort allowance. The local constructor correction remains outside
this frozen Windows packet.

The actual WebView2 container loaded resources, submitted the login form,
retained its Cookie through the 303 redirect and navigation, and exchanged exact
text and 37-byte binary WebSocket messages. Normal closure used code 1000 with
`wasClean=true` and SDK `failed=false`. Seven gateway denials left the independent
target request count unchanged. A separate renderer supplied real positive
storage controls; content attempts left its localStorage, Cookie, IndexedDB,
synthetic file and provider configuration intact. Generic `chrome.webview`
presence and a successful `postMessage` call were recorded separately from the
management permission assertions.

The real target observed a pending request before gateway close. Close plus
module-owner settlement took 0.646s within the unchanged five-second bound;
the same renderer then observed failure before its own host closed. Independent
qualification checked 22 raw files (192,400 bytes), 60 events, all five created
and closed loops, and all six registered servers. This includes the early
content gateway removed from the final cleanup list: its HTTPS attached-client
count went from two to zero before loop close. Early and final server snapshots
had zero attached clients and listeners, with zero callback failures/overflow
and empty outer callback stderr. Both hosts recorded actual engine-exit events
and joined input threads; original process handles and readers settled. The
original task instance ended and its exact registration was removed and proved
absent. This single-case grant is consumed.

The [Windows ACL/guest packet](../evidence/m119-windows-host-2026-10-25/acl-guest-source-preparation.json)
was explicitly authorized and staged: eight new files (59,423 bytes), reusing
the fixed SDK, host and 476 retained source/runtime files. Its
[first policy case failed before the renderer or policy assertions](../evidence/m119-windows-host-2026-10-25/acl-guest-runtime-qualification.json).
The provider SDK reported successful preparation and SQLite close, followed by
`MoveFileExW_no_replace` error 5 while publishing the new node directory. The
provider and original Python process both exited with code 1; their original
handles were waited, readers joined, and the original Task ended and its exact
registration was removed. The consumer, gateway and WebView2 host were never
created. The dependent guest case did not run.

The first fixed-log collector returned empty stdout because its compound
PowerShell input lacked the final empty line. The original failure is retained.
After automatic review initially rejected the collection and the user explicitly
authorized it, the same input plus one newline produced JSON. Exactly 13 existing
files (42,894 bytes) were collected from the 19 fixed candidates; six missing
streams belong to owners never created. Independent qualification checked the
original failure, Task/process records, and the two actual closed loops and one
server with zero listeners/clients before loop close and at final cleanup. This
failure-path accounting does not satisfy the planned four-loop/four-server full
policy case. No Task or SDK operation was retried, and no SDK/host/runtime was
rebuilt or recopied. The current directory-publication failure is localized to
the API boundary; its environmental cause and the causes of earlier failures
remain unproved.

Invitations in the unexecuted guest case bind identity, provider, Web service
and original deadline; separate provider origin configuration governs URL access.
The shared 2 MiB fixed-log allowance and original storage limits remain unchanged.

The [publication diagnostic](../evidence/m119-windows-host-2026-10-25/publication-diagnostic-qualification.json)
was explicitly authorized and ran once on the same frozen SDK. All four
same-parent `MoveFileExW` controls returned success, and SDK create reached
`AwaitingRecoverySave` before normal stdin EOF and original-handle settlement.
The SDK and outer Python process both exited zero with readers joined; the
original Task ended and its exact registration was removed. Thirteen fixed files
(35,652 bytes) were collected and independently checked, including all 140 SDK
diagnostic lines and the single ready record with no RPC/card export or recovery
acknowledgement. Source end hashes remained unchanged; no Native build occurred.

The sampler completed fourteen rounds with eleven acknowledged phase snapshots,
including four rounds in the SDK window. It observed one stable parent-directory
attribute/owner/DACL value and no SDK staging-directory name. The actual staging
ACL, its transient existence/removal and instantaneous external handles remain
unknown. This result is `UNLOCALIZED_SUCCESS`: it demonstrates success in this
new diagnostic context and does not repair or qualify the original failed policy
case. The original OS5 evidence, guest prerequisite failure and full M119 scope
remain open. The locked SQLx close path was checked against the original crate
checksums; that source evidence also does not establish the historical cause.

The [directory-event diagnostic](../evidence/m119-windows-host-2026-10-25/directory-event-diagnostic-qualification.json)
was explicitly authorized and executed once. It restored the original
PolicyTarget HTTPS/WebSocket prerequisites and kept the same target alive across
one SDK create. Four same-API controls succeeded. Independent parsing of the
982-byte native notification stream recovered fifteen events: the SDK staging
directory was added at sequence 7 and renamed at sequences 8/9 to `provider`.
The staging attributes, owner and inherited DACL were actually read while that
name existed and matched the fresh parent metadata. A later query of the old
name returned error 2 after rename; this is retained as unknown metadata and is
not a deletion event or evidence about the historical failed directory.

The SDK reached `AwaitingRecoverySave`, then exited zero after stdin EOF with
its original handle and readers settled. The single ordinary-user Task and its
original Python child ended zero; the exact Task registration was removed.
All eleven directory-I/O requests completed, including the final cancellation
completion, before the parent/event handles closed. Two TLS loops and their
one registered server closed with zero callback failures, listeners and attached
clients. Fourteen fixed files (55,486 bytes) matched remote hashes. Independent
Root qualification passed on the first read-only run without importing the
controller or its qualifier. Remote owned-root growth exactly equalled the new
payload, private state and fixed logs; existing raw counts and storage limits
were preserved. All current owned resources are terminal and the one-operation
grant is consumed.

This result remains `UNLOCALIZED_SUCCESS`. It closes the observed staging-name
and current-metadata gap for this diagnostic; it does not explain or repair the
original policy Os5, establish its historical ACL/handle state, or qualify the
unreached policy/guest and Windows/Linux role-exchange requirements.

No system ACL workaround or Windows SDK behavior fix has been qualified. The
original Windows SDK OS error 5, Win10054 callback and three attached-client observations
remain preserved; this new pass does not localize their original causes.
Whole-host egress and engine background destinations remain unqualified.
The later Windows policy/scoped-guest pass is recorded below. Windows/Linux
role exchange, remaining platform coverage and formal review remain open.
Plane remains authoritative for the ticket and its full
acceptance criteria; this record does not complete MIOPUNCH-119 or platform
convergence.

The [first observed full-policy attempt](../evidence/m119-windows-host-2026-10-25/observed-policy-first-run-qualification.json)
ran once on the frozen Windows graph after explicit payload/destination
authorization. All nine transports succeeded and all 22 fixed files (154,261
bytes) matched remote hashes. CREATE, JOIN, admission, verified TLS prerequisites
and the real WebView fetch/CSS/image checks succeeded. The case then failed
before the policy exercise: the page was complete, but its test marker was
absent. The initializer contained two statements while the pinned host wraps
evaluation input as one parenthesized expression. A source-pinned local
reproduction confirmed the syntax error; an IIFE candidate preserves the marker
and returns true. This local result is not corrected Windows acceptance.

The failed qualifier and original traceback remain unchanged. The guest gate
stayed closed. The Task, original Python child, two SDK processes and WebView
host settled; the exact registration was removed. All three loops and three
servers actually created before the failure closed without callback failures
or overflow. The complete four-loop/four-server scenario was not reached.
The separate directory observer completed all 15 requests, retained 20 events
and independently bound both SDK publications to their original owners.
Neither this observer result nor the absence of OS5 in this run establishes the
historical failure's cause or completes policy/guest coverage.

The [marker-corrected preparation](../evidence/m119-windows-host-2026-10-25/marker-corrected-policy-guest-preparation.json)
is now frozen. Its selected business class changes exactly one initializer into
an asserted single IIFE expression; every other class node, readiness check and
marker assertion is unchanged. The actual selected initializer and original
liveness expression passed the pinned host's boxing seam locally: the old input
raises SyntaxError, while the selected input returns true and leaves the marker
present. Current generated entry points and source pins also passed. Independent
Root verification checked the exact AST delta, 34 file references, original
commands, input and output hashes, and the Node stdin hash without repeating a
Windows case. Root's first verifier had a receipt-field comparison error; its
failed source and output are preserved, and only that verifier schema was fixed.

The six-file payload is 36,076 bytes. At preparation freeze, no new Windows
action had occurred and exact authorization/review was pending. The subsequently
authorized execution is recorded below; preparation alone did not qualify it.
The complete policy case gated the conditional scoped-guest/end case. The local
25 MiB evidence limit remained unchanged, with retained remote admission limits
of 1 GiB overall and 384 MiB for profiles.

The [actual marker-corrected Windows run](../evidence/m119-windows-host-2026-10-25/marker-corrected-policy-guest-runtime-qualification.json)
executed once after full payload/destination authorization and clarification of
the selected option. Both complete business qualifications and independent
directory observers passed. The full policy case proved real renderer resources,
target replacement, service off, member default denial, explicit allow restoration,
explicit deny precedence and cross-origin redirect denial. Live sockets closed
in 0.085, 0.063 and 0.056 seconds against the unchanged ten-second bound. The same
renderer and original SDK owners remained alive through the assertions; fresh
HTTP/WebSocket attempts had no target effect, including the original backoff.

The conditional guest case proved pending/duplicate-identity denial, an
independent same-network provider positive control followed by provider denial,
provider origin restrictions on another independently live target, and the
original identity/provider/deadline binding. Explicit guest end was delivered
and saved; the actual socket closed in 0.406 seconds, with zero SDK flows and
no subsequent HTTP/WebSocket target effect. This Windows batch did not execute
other-service denial, issuer revoke or natural expiry.

Independent Root qualification passed on its first actual read-only run without
importing the producer or qualifier and without repeating any test. All fourteen
transports ended zero; all fifty fixed files (501,089 bytes) matched remote
hashes. Both ordinary-user Tasks and original Python children ended zero and
their exact registrations were removed. All six SDK processes and two WebView
hosts settled with readers joined; each case closed all four created loops and
four servers without callback failures, overflow, listeners or attached clients.
The observers retained twenty and twenty-six native events, completed all fifteen
and nineteen I/O requests, closed both handles and joined their threads. Eight
same-API controls and six original SDK publications were independently checked.

The final retained remote root was 549,277,372 bytes, including 115,286,947 profile
bytes; the actual local raw roots held 24,188,415 bytes. The full reserve including
Root metadata remained 25,812,314 bytes within 25 MiB. All ownership is released
and this one-shot grant is consumed. The original marker failure and historical
SDK/transport failures remain intact; this pass does not establish their causes.
Windows/Linux role exchange, remaining platform/held-Native coverage, combined
management-host integration and formal review remain open. MIOPUNCH-119 and the
full non-UI implementation goal are not complete.

The [partial Windows/Linux role run](../evidence/m119-windows-host-2026-10-25/role-exchange-partial-run-qualification.json)
passed the repaired staging guard, all six transfers and their verification,
but its first direction failed before role business; the reverse was not run.
A later read-only check found the exact task absent, its controller journal and
entry receipts absent, and reproduced the original XML encoding error. Removing
the declaration alone exposed a second invalid task setting; no corrected parse
is claimed in that diagnostic. The Linux SDK exited normally, while its control
worker ended with a Python stdin-reader finalization error. Original failure,
process and diagnostic evidence remain pinned; MIOPUNCH-119 remains open.

The [frozen role repair review](../evidence/m119-windows-host-2026-10-25/role-exchange-reader-proof-review-rejection.json)
contains the independently verified source packet and the actual automatic-review
rejection. The reviewer did not recognize the previous broad authorization as
covering this new payload and destination; no process, transfer or role test was
created. Both directions remain unqualified pending a reviewed execution.

A [subsequent review](../evidence/m119-windows-host-2026-10-25/role-exchange-review-permission-rejection.json) is recorded in the adjacent evidence file: after the user allowed test review, the reviewer again required specific authorization for the six-file transfer and destination. No process or transfer occurred.

The [specifically approved role run](../evidence/m119-windows-host-2026-10-25/role-exchange-specific-approval-first-run.json) passed staging and verification, then its first direction failed before business qualification. The Windows task controller returned `RuntimeException` without terminal proof; the Linux SDK exited normally and the repaired control reader joined. The reverse direction did not start.

The [subsequent task diagnostic](../evidence/m119-windows-host-2026-10-25/role-exchange-task-diagnostic.json) found the exact Windows task, its journal and entry records absent; the original task definition passed its guard in memory. A local PowerShell comparison reproduced an unset script-scoped journal counter before the first append, and the scoped initialization passed the same comparison. Both role variants then passed local real-file journal, byte-boundary and first-exception checks. This supports a pre-registration controller defect, but the original remote exception message was not captured and its historical cause remains unproven. Neither role direction is qualified by these local checks.

The corrected fresh batch passed local preparation and capacity checks. Automatic review rejected its remote execution before process creation because it required specific authorization for the new payload and destination.

After specific authorization, the first direction ran but failed before the Windows SDK started. Its new Task reached terminal state and its registration was removed; the entry recorded `RuntimeException` without a detailed cause. The reverse direction was not run.

Local PowerShell 5.1 tests identified a separate entry precondition defect: the original property-count expression rejects even the valid 476-file manifest. Explicit array counting accepts 476 while rejecting 475 and 477 for both roles. The first failing condition on the remote machine remains unknown.

A local repair retains that assertion and records the failing phase and error code. Both roles passed the candidate's prelaunch scenarios, including early manifest failures; filesystem and process boundaries in those scenarios were fixtures, so the remote business remains unqualified.

The next authorized first-direction run reached both SDKs but the Windows admission request returned failure; the message was omitted. New task and SDK owners settled, and the reverse was not run.

The retained Windows diagnostic stream subsequently identified one failed
`CONTROL_CONNECT` on that first direction. The request was not accepted and the
joiner remained outside approval; no Web HTTP or reverse-direction assertion was
reached. The fixture disabled relay, public contact, and member contact, leaving
direct IP candidates from the fresh invitation. The provider advertised eight
addresses, but the diagnostic did not identify which address was attempted or
establish a reachable cross-host path. A current interface snapshot and a local
route timeout are therefore hypotheses, not a diagnosis of the Windows failure.

A bounded local feedback case exercised the same restricted admission control
entry with one owned UDP socket that silently drops packets and one real loopback
server address. With the blackholed address first, the original run passed the
5-second caller deadline and hit the 7-second scenario timeout, failing the
core connection assertion;
the paired single-address positive passed. After allocating each inactive direct
address a share of the *remaining original deadline*, both cases passed, as did
the three existing active-path commit/rollback tests. The route-specific attempt
uses a child cancellation token and keeps bootstrap packets scoped to its one
address. Raw source snapshots, build receipts, pins, stdout, and stderr are
retained under `/tmp/m119-route-feedback-20261028/`; the original failing exit
and assertion remain intact. This proves the local starvation repair, not a
Windows/Linux role exchange or MIOPUNCH-119 completion. A same-operation Windows
run with a verified current SDK and a genuinely reachable direct path is still
needed to classify the retained `CONTROL_CONNECT` failure.

A separate, bounded transport precheck then confirmed one cross-host UDP path:
an owned Windows socket bound to `100.64.10.8` received a 24-byte nonce sent
from the Linux host's `192.168.4.5` address and echoed the exact bytes to that
socket. Both owners exited normally in under a second; the local record is
`/tmp/m119-route-feedback-20261028/route-probe.json`. This establishes UDP
reachability on that pair of addresses at the probe time. It does not show that
the failed invitation selected that address, that QUIC can complete, or that the
later Windows admission succeeds.

The [route feedback qualification](../evidence/m119-windows-host-2026-10-25/route-fair-feedback-qualification.json)
records the original failing assertion, fixed local admission checks, immutable
current Windows SDK, and fresh first-direction test packet. The Windows build
reused the shared target with 526 Fresh artifacts, two project targets rebuilt,
and no dependency rebuild. The prepared transfer totals 189,844,855 bytes,
including a 189,760,518-byte SDK, into the existing laptop test root; its
destination was absent in a read-only precheck. Automatic approval review first
rejected this new payload. After the user explicitly approved it, execution
stopped in a local PowerShell framing assertion before any SSH or SCP process
started. Its original `STOP` and empty owner sets remain preserved.

The [fresh packet qualification](../evidence/m119-windows-host-2026-10-25/route-next-preparation-qualification.json)
records the corrected, locally checked first-direction packet with a new nonce.
Its test files have different bytes, although the SDK, destination, scope, and
total transfer size are unchanged. Automatic approval review rejected this
distinct payload before process creation because the earlier exact approval did
not cover the new test-file contents. The fresh packet's runtime and private
batch are absent. The Windows admission outcome remains the original failure
until this exact fresh packet receives specific authorization and runs.

The authorized fresh packet transferred and verified the current Windows SDK,
then its stage guard stopped before any test-file transfer or Task creation.
A read-only census found 927,130,523 bytes in the owned remote root; the old
two-direction reserve of 218,188,145 bytes exceeded its 1 GiB limit. A scoped
cleanup removed an older SDK deployment copy after confirming no matching
process or running M119 Task, retaining its local immutable pin and raw history.
The immediate receipt measured 187,162,624 freed disk bytes, and a post-cleanup
census verified the other SDK hashes and capacity. A further entry review then
found the removed copy is required by the retained 476-file base manifest. The
[cleanup erratum](../evidence/m119-windows-host-2026-10-25/route-cleanup-erratum.json)
records this mistake and its exact path/hash. After the user specifically
authorized restoration, the [restoration qualification](../evidence/m119-windows-host-2026-10-25/route-cleanup-restoration-qualification.json)
confirmed the original 187,160,555-byte SDK at the original path with the
required hash and unchanged base manifest. A separate read-only census then
verified all four SDK hashes and 37,475,059 bytes of margin beneath the original
1 GiB root cap when reserving only the authorized first direction. This restores
the old entry prerequisite; no Task or admission operation ran during the
restoration. The current first-direction packet uses the already verified new
SDK and 84,338 bytes of fresh test files, with both stage and pre-run guards
scoped to that one direction; its [preparation record](../evidence/m119-windows-host-2026-10-25/route-reuse-preparation.json)
pins the exact packet, retained failure, and capacity margin. After explicit
authorization, it passed both remote guards, transferred and verified all six
test files, and parsed both Task definitions before running the first direction.

The original run reached `AwaitingApproval` on Windows, and Linux observed
exactly one request from that applicant. Both native SDKs and the exact Windows
Task exited; the Task registration was removed and the Linux control reader
joined. The old full-Web qualifier stopped at its final stderr assertion because
it requires the older SDK11 initialization marker, which this current SDK build
does not contain. Successful admission emitted an empty Windows node stderr.
Its admission-only worker reports `business=false`, so the original
`close_endpoint` assertion expecting a full business `ok=true` also failed
despite terminal owners and empty cleanup error lists. The original `STOP`,
false worker results and all 26 collected raw files remain intact. The
[read-only admission qualification](../evidence/m119-windows-host-2026-10-25/route-current-admission-qualification.json)
checks the original core assertions, native publication, loop closure, Task,
readers, and raw hashes against that same run. This qualifies the first-direction
admission control path only; Web business, reverse direction and full MIOPUNCH-119
remain open.

The [current first-direction full-Web packet](../evidence/m119-windows-host-2026-10-25/full-route-current-preparation.json)
is prepared for the remaining role business check. It reuses the verified
Windows SDK, uses a newly pinned current Linux SDK for provider sync/return
traffic, and would transfer 84,357 bytes of fresh test controllers/Task files.
The frozen operation includes approval, signed synchronization, real HTTPS
resources, redirect/login/Cookie, bidirectional WebSocket and management
isolation. Its stage and pre-run guards reserve only this one authorized
direction under the unchanged 1 GiB remote root limit. It has not run; the
reverse direction and full MIOPUNCH-119 remain unqualified.

The first authorized full-Web run passed remote staging, definition parsing and
the first-direction guard, but stopped at its inherited provider-approval
assertion before synchronization or Web exercise. The provider returned
`Approved`; the test expected `Complete` immediately. In the current SDK,
approval persists phase 1 as `Approved`, and `Complete` is phase 3 after the
consumer receives admission material. The [original failure qualification](../evidence/m119-windows-host-2026-10-25/full-route-approval-stage-failure.json)
pins the wrong assertion, actual public stages, 26 collected raw files and
terminal owners. The fix belongs to the shared role-test assertion; this run
provides no full-Web business acceptance or reverse-direction evidence.

A [fresh corrected first-direction packet](../evidence/m119-windows-host-2026-10-25/full-stage-fixed-preparation.json)
now uses the same current SDK pins and shared role operation. Its sole stage
assertion change expects provider `Approved` and retains the subsequent
consumer `Complete` check. The new 84,363-byte controller/Task payload and
one-direction capacity budget are frozen; no runtime or remote Task has been
created for that packet at preparation time. It subsequently received specific
approval and passed remote staging, definition parsing, and both guards.

The corrected run then reached provider `Approved` and consumer `Complete`.
Linux provider `service_sync` returned `saved_locally=true`, but the Windows
consumer's reverse `service_sync` returned a generic operation failure; the CLI
did not expose the failing phase. The original controller and worker results are
failed, the Web exercise did not begin, and both SDKs and the Windows Task are
terminal. The [independent sync-failure qualification](../evidence/m119-windows-host-2026-10-25/full-stage-fixed-sync-failure.json)
verifies all 26 raw files and preserves this first failure. A fresh local
manual-admission pair using the same pinned Linux SDK passed both immediate
sync directions over loopback. That narrows the investigation but does not
prove the Windows-to-Linux route or Windows bundle processing. No full-Web
business or reverse-role acceptance follows from this run.

The [next one-route diagnostic packet](../evidence/m119-windows-host-2026-10-25/single-route-diagnostic-preparation.json)
is prepared but unexecuted. It retains the same admission and Web assertions,
and changes only the Windows consumer's reverse service-sync target to the
Linux provider's freshly advertised `192.168.4.5` address. The local preflight
and read-only Windows capacity/collision check passed. Retaining the failed
run's evidence required an explicit 524,288-byte increase to this packet's
local evidence cap; the remote 1 GiB root cap, pinned SDKs, and build graphs
are unchanged. Running this new packet requires specific authorization for its
84,369-byte payload and remote destination.

The specifically authorized single-route run passed both signed service-sync
directions and reached real Windows WebView2 content: HTTPS resources, login,
redirect, and bidirectional WebSocket assertions passed. Its isolation page
eventually reported every denial and storage boundary as true. The run still
failed at the shared provider `unchanged` assertion. That assertion compared
the entire `web_status` value captured before business with the value after
12 completed Web flows; `completed` changed from 0 to 12 while
`service_status` and the Web configuration fields stayed equal. The [original
failure audit](../evidence/m119-windows-host-2026-10-25/single-route-original-shared-assertion-failure.json)
pins all raw files and this exact difference. Its Windows worker retained a
stale earlier oracle label, so that label does not identify the failing
assertion. The later backoff and final isolation checks did not run, and the
reverse role remains untested. The shared test assertion must compare durable
configuration while checking completed-flow counters separately before a fresh
full-role run can qualify MIOPUNCH-119.

The shared assertion fix has a captured-value regression: the original code
fails on the observed 0→12 completed-flow change; the corrected code passes
that value while rejecting a changed origin, changed service state, and no
flow progress. Two [exact role packets](../evidence/m119-windows-host-2026-10-25/shared-config-fixed-two-role-batch-preparation.json)
are now prepared for one approval: Linux provider/Windows consumer first, then
Windows provider/Linux consumer after a fresh remote capacity check. Both use
the same corrected worker, existing SDK pins, and original Web assertions.
They total 170,074 bytes of new controller/Task payload and have not run.

A separate [five-address local feedback case](../evidence/m119-windows-host-2026-10-25/local-five-address-sync-feedback.json)
used the current Linux SDK and fresh manually admitted nodes. Its JSON put
three nonresponding UDP addresses before a reachable fourth, and reverse
`service_sync` took about 49 seconds. A [controlled comparison](../evidence/m119-windows-host-2026-10-25/five-address-canonical-order-comparison.json)
clarifies the actual order: Iroh stores addresses in an ordered set, so all
four `127.0.0.1` blackholes preceded the reachable `127.0.0.2` address even
when JSON listed it first. That unchanged canonical order again took about
49 seconds. Putting the reachable address first in the ordered set took
0.98 seconds with the same five-address public operation; both nodes saved
and closed cleanly. This isolates local cold-address ordering latency, not
the cause of the original Windows-to-Linux failure.

A later [read-only Windows route query](../evidence/m119-windows-host-2026-10-25/windows-five-linux-address-routes-readonly.json)
for the five Linux announcements found that the fourth address,
`192.168.4.5`, has a specific overlay route from Windows, while the second
selects Windows' own virtual-interface address and the other three select the
default gateway. This is route-table evidence only; it does not prove which
UDP or QUIC attempt failed in the original run. The original multi-address
SDK call still needs product-level diagnosis and regression before M119 can
be closed.

Before running the first shared-assertion packet, its local continuous admin
ledger guard found 481,181 bytes against a 480,136-byte bound. No remote action
occurred. The original failure and packet remain retained. A reviewed local
budget adjustment produced a new first-role packet; its normal execution
request was rejected by automatic approval review because the earlier broad
authorization was visible only in session-file output, not as trusted
authorization for that exact new private payload and destination. No process
started. The [revised two-role batch](../evidence/m119-windows-host-2026-10-25/shared-config-fixed-two-role-budget-v2-batch-preparation.json)
freezes both new payloads (170,114 bytes total) for one current user approval.
Its SDK pins and remote 1 GiB cap are unchanged; neither packet has run.

The user then explicitly authorized that exact revised two-packet batch, and
both packets ran once. The first passed admission, signed sync and real HTTPS
resources/login; WebView2 reached navigation. Its controller mail helper then
read an existing but empty JSON message and failed with `JSONDecodeError`;
the [original failure record](../evidence/m119-windows-host-2026-10-25/shared-config-first-original-mail-failure.json)
retains all collected raw files. The reverse packet started its Windows Task
but the provider SDK `create` exited before readiness with Windows error 5;
Linux consumer and Web business never started. Its [original failure record](../evidence/m119-windows-host-2026-10-25/reverse-role-original-windows-create-failure.json)
and [read-only ACL observation](../evidence/m119-windows-host-2026-10-25/reverse-create-code5-path-acl-readonly.json)
keep that result distinct from the mail failure. The parent directory grants
the Task user full access and the destination node directory is absent, so
the specific Windows error remains unlocalized in this run.

The shared mail reader had only waited for the final JSON filename to exist,
while the writer created that name before finishing its write. A controlled
same-seam test reproduced the empty-file error, then passed after both mail
directions published a zero-byte commit marker only after the payload was
closed; missing payload, wrong nonce and missing marker still fail. The
[new two-role packet batch](../evidence/m119-windows-host-2026-10-25/mail-commit-two-role-batch-preparation.json)
retains the original Web assertions and existing SDK pins. Its two new
payloads total 171,542 bytes. At preparation, they had not run and required
a fresh exact authorization because their bytes differed from the completed
batch.

The authorized mail-commit first role ran once. Admission, sync, HTTPS
login/navigation and native WebSocket passed, then Linux timed out at
`snapshot` near 100 seconds and Windows stopped in content isolation. All
owners terminated; [raw evidence](../evidence/m119-windows-host-2026-10-25/mail-commit-first-timeout-and-reverse-capacity.json)
records the incomplete result. The reverse packet did not run because the
root plus its 104 MiB reserve exceeded the unchanged 1 GiB cap by 9,015,435
bytes.

A [new frozen batch](../evidence/m119-windows-host-2026-10-25/window180-v2-two-role-batch-preparation.json)
contains 171,548 changed test bytes. It retains business assertions, gives
both roles one 180-second deadline, moves Task observation to 190/200 seconds,
and tightens each profile cap to 24 MiB with a 64 MiB case reserve. Local
checks and read-only baseline capacity checks passed. The first packet then
passed full one-direction business and ownership qualification, with 32 raw
files verified. Reverse capacity passed after it settled. The original reverse
packet failed its local budget preflight before remote action because first-role
raw files were counted as metadata. The [corrected reverse packet](../evidence/m119-windows-host-2026-10-25/window180-v3-reverse-preparation.json)
then ran once: Windows provider creation and admission succeeded, but Linux
consumer stopped when its test worker sought a pinned sandbox helper beside
the copied worker instead of at its recorded source path. The [original
failure](../evidence/m119-windows-host-2026-10-25/window180-v3-reverse-sandbox-pin-failure.json)
and all 26 raw hashes are retained. A [new packet](../evidence/m119-windows-host-2026-10-25/window180-v4-reverse-preparation.json)
fixes the fixture lookup. A [local preflight](../evidence/m119-windows-host-2026-10-25/window180-v4-local-fixture-preflight.json)
reproduces the v3 failure and passes v4 with negative pin checks. The
user-authorized [v4 run](../evidence/m119-windows-host-2026-10-25/window180-v4-reverse-result.json)
then passed staging, Windows provider creation, admission and service sync.
Linux consumer stopped while starting its private desktop: the test output
quota wrapper rejected the owned xauth helper's output capture. All 27
collected raw hashes match; the Task and SDK nodes terminated, while the
xauth owner journal retains an assertion error in capture cleanup. Reverse
page/WebSocket/isolation business was not reached. The user directed stopping
after this one run, so no fix, retry or post-run remote capacity query followed.

On the later request to continue, the xauth capture failure was reproduced at
the output quota seam. `web_process_owner` now creates each capture file once
and appends only newly received bytes when a timeout's partial output is
completed; repeated cleanup does not truncate retained evidence. The original
quota wrapper accepted partial/full/repeated capture, a real child and the
actual Linux xauth, Xvfb, authenticated display probe and private bus chain;
all owned processes terminated. The existing owner test suite passed 8 tests.
The [frozen v5 reverse packet](../evidence/m119-windows-host-2026-10-25/window180-v5-reverse-preparation.json)
contains this source pin and 86,137 bytes of changed payload. Its local pin,
budget and fixture checks passed; a fresh read-only Windows check found no
path collision and 1,059,557,134 bytes including reserve under the unchanged
1 GiB root cap. No v5 payload has been sent or run, so reverse Web business
and whole M119 remain unaccepted.

The resumed reverse investigation reached the complete host chain. It found
that Linux Unix socket paths also include WebKit's generated proxy suffix,
and that WebKit passes nested Bubblewrap options through `--args FD`.
The fixture now uses short per-host private directories, checks the generated
socket path, and expands bounded inherited argument files before counting and
limiting tmpfs mounts. The maintained packet checker reproduces the old
management-control path failure using the deployed path length and passes the
corrected packet. Real local content and management-control hosts also passed.

The v7 run reached all business and isolation assertions but blocked while
writing its final report above the original 88 KiB hard limit. Its failure was
preserved; only the original blocked test worker was terminated after checking
its launch time and exact command through a pidfd. That run is not accepted.
After verified source deduplication and budget redistribution within the same
aggregate cap, the [v8 reverse run](../evidence/m119-windows-host-2026-10-25/window180-v8-reverse-qualification.json)
passed full Web business, management isolation, and both endpoints' owner
cleanup. All 40 raw files (158,257 bytes) were independently hash-verified.
Linux output used 96,872 of 131,072 bytes with no quota failure or pending write.
This closes this reverse runtime gap; it does not by itself accept whole M119.

Two Rust cleanups remove a full WebSocket-message clone used only for validation
and release the global Web state mutex before serializing completed HTTP results.
The library check passed. The accepted reverse run reused the prior immutable
SDK, so it does not constitute runtime validation of these Rust cleanups.

The [current-source Linux runtime](../evidence/m119-windows-host-2026-10-25/current-rust-linux-runtime-20261029.json)
subsequently built and pinned the current SDK once and passed the real isolated
WebKit HTTPS scenario, including login/navigation, WebSocket, management
isolation and pending-request close (0.264 seconds). Its process/readers and
owned loop cleanup passed. This supplies Linux runtime evidence for the Rust
cleanups; the corresponding current Windows artifact qualification is pending.
