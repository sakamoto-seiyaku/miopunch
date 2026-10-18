# Browser restricted admission

MIOPUNCH-94 migrates the browser SDK to the endpoint contract accepted in
ADRs 0010 and 0012. The ordinary library contract is a separate concern.
The browser continues its existing protocol and IndexedDB storage format.

`BrowserNetwork` owns a `RestrictedEndpoint`. Its incoming dispatcher classifies
`RestrictedAccepting` without running admission, then selects either control
delivery or business delivery. `BrowserGate` implements `EndpointAdmission`;
the ordinary hook retains only its existing pre-connect veto.

Pending sessions record authenticated peer, ALPN, side and weak revocation
authority. They cannot upgrade to ordinary connections. SCRAM proof and the
existing one-byte member-stage exchange precede shared session preparation.
Requests retain their existing MUC1 confirmation. Member and replica delivery
require the shared prepared path, current authorization, fresh path checks and
owned final confirmation. Only `business_released` publishes an ordinary weak
connection and registers member maintenance.

Admission requests and material delivery use `ControlOwner` and bounded
`ControlExchange` messages. Fixed-size segments and bounded length-prefixed
history preserve the existing invitation, approval, AUTH/SAVE/DONE and history
bytes. Browser storage still commits authorization before the admission secret;
delivery retries retain the original identity and request.

Both initial RTC preparation and active improvement use confined control
handles and the existing bounded JSON wire. The original session identity,
exporter binding, permission generation, exact route, bilateral readiness and
fresh final ACK remain checked. Pending finalizers retain only a confined
handle; long-lived pumps and gate metadata use weak revocation authority.
Ordinary stream I/O remains available after business delivery.

Validation uses actual SDK callers on owned loopback IRC TLS/WSS and a local
data relay, with fresh synthetic identities and a fixed M93 Native counterpart.
The JavaScript consumer pinned before the first browser edit is unchanged.
The complete request, single approval, durable material save, identity reopen
and member PING/PONG story passes on the migrated SDK. Initial RTC and an
existing member's Relay-to-RTC improvement pass as well.

Additional browser callers exercise incoming manual approval, precode delivery
and idempotent usage, persistent applicant reopen, incoming member streams and
replica synchronization. A browser-to-browser improvement verifies incoming
active-control dispatch and both RTC roles on the original member connection.
Its explicit `PreferDirect` fixture policy does not change the default policy.

Affected negative cases cover absent RTC APIs, disabled resources, cancellation
and deadline at an opened but unqualified RTC channel, and public local
departure after qualification but before final delivery. Local departure
prevents browser business delivery and retires RTC authority, routes and pumps.
A counterpart that has not received that departure still uses its own current
authorization view, within the propagation window described in `CONTEXT.md`.

SDK close, WebSocket closure before context destruction, Native exit, relay
exit and container removal are observed. Initial and later socket observations
are retained separately. Failed intermediate runs remain recorded. WASM with
RTC builds successfully; the default browser graph and the affected Native
shared-library graph pass their compiler checks with pinned dependencies and
the shared repository target. The existing 249 workspace files are unchanged.

Receipts, failure history, review reports and the checked raw archive are in
[the evidence directory](../evidence/browser-restricted-admission-2026-10-19/README.md).
This evidence does not qualify the separate public/physical test matrix or
Android throughput and power behavior.
