# p2panda transport resolution compatibility

This implements ADR 0012's ordinary-default rule for p2panda-net. It does not
change membership, resource authorization or the restricted connection pipeline.

`Endpoint::connect` and `connect_with_config` again return the existing
`TransportInfoMissing` error when the address book has no iroh transport entry.
This includes an absent node and a known node with no transport information.
An address-book read failure remains an address-book error and never starts an
iroh lookup. Existing transport entries continue through the original connect
path; a transport entry containing only an identity retains upstream's own
iroh lookup behavior.

The added `connect_resolving` and `connect_resolving_with_config` methods opt in
for each call. Both ordinary and restricted wrappers expose them and share the
same address-book read, connection actor and transport configuration. Only a
missing transport entry permits an identity-only fallback. Installing a builder
hook does not select resolving, and no new service, registry or compatibility
mode is introduced. Iroh retains ownership of its configured lookups, resource
checks and cached addresses. A lookup may wait for later information; callers
retain their existing timeout/cancellation mechanism. This change does not add
a p2panda actor cancellation guarantee.

The root wrapped probe explicitly uses resolving for its configured memory
lookup. It awaits the existing raw endpoint's close after a successful exchange
or a failed send. The restricted wrapper fixtures also opt in, retaining real
business delivery, admission checks and revocation through the same actors.

The existing connection actor's rejection classification is retained. A local
pre-connect or post-handshake rejection still fails the call, without marking
the peer's address as unreachable. Genuine lookup and handshake failures keep
their distinct errors and normal reporting. The existing rule suppressing
failed-connect health reports when the local endpoint has no globally reachable
IP also remains. The public error enums are unchanged; the tests compare the
existing error families and messages rather than introducing another error API.

The public `DiscoveryBackoffConfig::bounded` now documents and validates its
actual sampling representation: both increment bounds are positive, exact whole
milliseconds, at most `u64::MAX` milliseconds, and `min < max`. The interval is
half-open. Fractional bounds previously could collapse to an empty interval
after truncation, and very large values could truncate during conversion.
Invalid bounds now return `InvalidInput` before sampling. Initial value and
ceiling retain full `Duration` precision, with initial value no greater than the
ceiling. Saturating addition preserves the existing immediate clamp at the
ceiling even near `Duration::MAX`. Default ranges, random sampling and reset
policy are retained; the old one-step overshoot is not restored.

The compatibility baseline uses the exact p2panda v0.7.1 source at official
commit `083b48215964e92a564f3c83a1c607b00e94aa64`, with the existing pinned fork
dependency graph (including iroh 1.2.0). A temporary manifest flattens upstream
workspace inheritance and gives that local source selection a package alias;
the library source and ordinary consumer bytes are unchanged. Only local
selector records differ in the lockfile; foreign versions, sources, checksums
and dependency rows are preserved. This is a differential p2panda source
baseline, not a claim that all dependencies are an unmodified upstream SDK.

Before the first production edit, the fixed consumer returned
`TransportInfoMissing` in about 30ms in an isolated container. The fork timed
out after 751ms with the same consumer. A separate configured-lookup regression
observed one unexpected lookup; backoff regressions observed acceptance of
`1ns..2ns` and duration-addition overflow. The consumer at
`tests/p2panda_consumers/ordinary_missing_transport.rs` retains those qualified
bytes. The M93 hook consumer is a historical fork-extension fixture that relied
on the former implicit fallback; its frozen bytes are retained rather than
rewritten or presented as an upstream p2panda consumer.

Public regression tests use owned loopback endpoints, complete PING/PONG over
both resolving methods, and close the endpoints. A separate Docker
`--network none` case assigns `11.0.0.1/32` only within its own namespace so the
existing global-connectivity guard actually permits health reports. With no
default route, it verifies that both local rejection phases preserve health
while a genuine unsupported-ALPN handshake failure marks the peer stale. Store
failure and an owned failing lookup exercise their actual public entry points.
The scoped proof and SDK receipts are in
`docs/evidence/p2panda-resolution-compatibility-2026-10-19/`.

These checks cover this adapter and the affected SDK/probe flow. They do not
replace the separate public-network, Android-device or full transport-matrix
acceptance work.
