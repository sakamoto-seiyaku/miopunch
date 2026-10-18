# Native restricted admission evidence

MIOPUNCH-93 migrates the Native SDK, resource assembly, ControlSession,
CLI/probes and Native fixtures to the independent restricted admission API.
The implementation builds on `b0cbabec640649873fcef3bf558bde89f6baf0db`.
The implementation commit is `fcaf11b71c708780295d6d98f9955f28cc34dccb`.
The [implementation note](../../notes/2026-10-19-native-restricted-admission.md)
describes ownership, existing application framing and the migration boundary.
The 249 pre-existing workspace files retain their original hashes.

## Acceptance results

[validation.json](validation.json) preserves build receipts, runtime results,
compiler contracts, fixed counterpart hashes and cleanup outcomes. Each run
belongs to its captured source snapshot; earlier passing snapshots are not
relabeled as the final source. The final snapshots include formatting and
compiler-guided import cleanup. The default-feature binding correction only
moves the mutable permission variable after the feature-specific expression.

The public Native SDK story exercises a real awaiting-approval request, one
approval, durable authorization and admission-secret delivery, persistent
identity reopen and an ordinary member PING/PONG stream. The 11-case Native
admission target also covers MQTT control ownership, refusal of signaling-only
business, rotation, recovery, replica synchronization and maintained startup.
All 11 cases pass in the final Native test build with an owned MQTT broker.

The broader Native SDK selection qualifies 116 test entries across manual
admission (15), precode (12), management recovery (17), genesis recovery (5),
network migration (5), TCP direct (11), resources (26), contact listener (3),
contact priority (6), maintained startup (5) and Native admission (11). Manual
admission's unchanged 14 passing entries and corrected crash case come from
separate receipts; the five migration and 26 resource cases pass in their
corrected complete targets. This count includes the pre-existing inert worker
entries; it does not mean 116 independent end-to-end SDK stories.

The final core targets pass: restricted endpoint (19), application protocol and
immutable Native counterpart interoperability (4), ControlSession handoff (3),
and restricted p2panda wrapper (1). They cover absent or rejected decisions,
current permission checks around ordinary hooks, cancellation and deadlines,
owner drop while helpers are retained, foreign path proofs, post-handoff
revocation and ordinary endpoint early-data behavior.

The 18 compiler consumers all meet their expected outcomes. Ordinary Native
member links compile. The corresponding control consumer produces exactly
four `E0308` errors at the actual member synchronization, replica, path-update
and message entry points. The other restricted consumers cannot borrow or
convert ordinary endpoint/connection state, open business streams/datagrams,
upgrade weak handles or use early data. The immutable ordinary p2panda
consumer compiles unchanged and completes its original hooks, PING/PONG and
natural close.

Native all-target checks pass with default features and with RTC plus the
replica, signaling and NAT fault-test features. The current browser WASM SDK
and bindgen package build successfully against the shared control modules.
The late Native control-fixture results are included by target in
`validation.json`; gateway and traversal workers retain their explicit lab
entry requirements rather than being counted as physical network trials.
The complete late targets pass: MQTT bootstrap (8 entries, including its inert
worker entry), path maintenance (19), public resources (11) and negotiated
accept context (3). Owned brokers are provided to their MQTT-dependent cases.

## Fixed browser interoperability

Three complete replays pair the current Native SDK with the immutable,
unmigrated M92 browser WASM package. Each uses fresh synthetic identities,
owned loopback Ergo TLS/WSS and an owned local HTTP relay. Each completes
approval, saved authorization and secret, identity reopen, member PING/PONG
and natural SDK shutdown.

| Replay | Actual data path and original-session evidence |
| --- | --- |
| Relay | Both sides select the permitted relay; the common coordinator records the original binding and fresh ACK. |
| RTC | The member connection selects the existing RTC custom carrier; relay is not its selected business path. |
| Active improvement | The same living member connection improves from relay to RTC, with both sides Ready and fresh ACK, under the existing explicitly selected PreferDirect policy. |

The active replay does not change the product's default quality policy. An
earlier default-quality relay-loss experiment remains unqualified; its private
failure and public execution receipt remain preserved locally. It is not
presented as a successful recovery case.

Native processes exit zero, browser WebSockets close before the browser
context, RTC associations and pumps retire, owned origins close, relays exit
normally and containers are removed. Synthetic private keys and NSS stores
were removed after actor absence was verified. Published SDK results contain
an explicit allowlist of outcomes; private SDK identities, stores and detailed
signaling logs are excluded. These controlled stories do not extend the
existing independent public-network or physical-device qualification matrix.

## Original paths and preserved failures

Affected regressions retain real physical socket/source-IP ownership, the
original connection, fresh probes and path retirement. The scoped physical
UDP and legacy NAT selections pass 11 and 23 cases respectively. RTC
qualification (7), release validation (1), revocation barrier (1), actual RTC
control release (3) and the wrong-offer-role case also pass. The RTC seal case
keeps an empty type-wide custom allowlist: the original actor validates only
the current exact connection/address seal, without widening the carrier type.

The raw archive retains the expected missing-interface compile failures and
the runtime failures that drove the migration. It also retains unsuccessful
harness iterations: missing selector dependencies, ownership errors, premature
stream reset, and mutable bindings evaluated under different feature sets.
Selectors changed only local package records and direct use of already-pinned
dependencies; external lock rows were not re-resolved.

The late MQTT fixtures still used custom ALPNs for scenarios requiring the
common coordinator's path-preparation protocol. Their corrected business entry
uses `MEMBER_ALPN` while retaining the same payload, identity, original-session
and path assertions. The cancellation fixture holds its first ordinary hook
after safe path checks, then allows the healthy retry. The two-broker case
selects each saved origin by disabling the other before a fresh endpoint takes
its policy snapshot; both independent brokers remain live. The existing node
assembly owns one standing MQTT contact, so it does not expect two simultaneous
standing routes. No production policy or protocol implementation changed for
these fixture corrections.

The disabled-UDP case verifies preflight rejection with zero connections
opened on either endpoint. Its earlier accept wait was forcibly bounded after
120 seconds because the planner correctly never dialed. The startup case
explicitly selects a permitted live contact while another saved service is
unavailable. The old assertion depended on random port ordering and assumed
automatic MQTT failover, which the existing single-contact assembly does not
establish. This result does not qualify automatic failover.

An unchanged baseline Native SDK and the unchanged original fixtures were also
built and run through the same legacy library entry points. They failed during
their earlier admission setup, before reaching these path scenarios. That
result establishes a stale fixture baseline; it does not prove that the later
custom-ALPN failures had the same cause. Both baseline and migrated failures
remain in the archive. The standalone baseline-manifest attempt failed with
`--locked`; the successful dependency selector retained the complete existing
foreign lock rows without re-resolution.

Several earlier failures were fixture lifetime or scope mismatches: ordinary
accept used for a control request, waiting for a driver while still owning its
endpoint, and a path already promoted to Data being opened through the wrong
API. The resource mDNS case originally hung and was forcibly stopped. Its
corrected fixture resolves the actually advertised address with the existing
lookup before the bounded request; it preserves the production coordinator's
pre-existing identity-only behavior. The complete corrected resource target
finishes naturally. These earlier failed or forced runs remain failed.

## Reproduction and review

[raw-verification.tar.gz](raw-verification.tar.gz) contains raw Cargo diagnostics,
test logs, receipts, frozen contexts, selected-input manifests, drivers,
selectors and compiler consumers' diagnostics. [archive-manifest.json](archive-manifest.json)
records every member's hash and size. Selected binaries, libraries and full
captured source copies remain under the local paths recorded by those receipts;
no Cargo target tree was copied.

Builds use the shared repository `target/`, cache lease, committed profiles and
pinned lock/source/checksum inputs. Failed source, tool, artifact or lock
preconditions stop before Cargo. Final tests verify their binaries and every
actual Cargo executable that a child-process test can invoke before and after
runtime. Every claimed successful runtime selects nonzero cases, passes its
guards and exits without forced termination.

[review.md](review.md) keeps the separate Standards and Spec reports. There are
zero hard standards violations, one accepted low-priority helper duplication
and zero actionable spec findings. Browser migration and restoration of the
remaining old library hook/early-data contracts stay with M94 and M95.
