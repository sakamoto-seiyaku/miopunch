# Ordinary endpoint compatibility after typed admission

This implements
[MIOPUNCH-95](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6ee0dbab-7d9a-445e-bb2a-ca54fe7ea28b)
on the M94 baseline `ba5f544a`, under the accepted M91 specification and
ADRs 0010/0011/0012. It restores ordinary library contracts after the separate
Native and Browser migrations. It introduces no new product policy or wire
protocol. The broader compatibility specification and independent public and
physical-device matrix retain their own scope.

## Interface and implementation

Ordinary `Accepting::into_0rtt` again directly returns
`IncomingZeroRttConnection`; outgoing `Connecting::into_0rtt` keeps its original
`Result`. The ordinary hooks implementation is byte-for-byte the fixed official
iroh 1.2.0 source: `AfterHandshakeOutcome` has only `Accept` and `Reject`, and
hooks retain their original order and veto behavior.

The unused transitional `Builder::admission_gate`, `control_only_alpns`,
`EndpointHooks::before_data_release`, expanded hook outcomes and obsolete
`InvalidControlRoute` error are removed. Current Native and Browser consumers
already use independent admission. Their similarly named
`NativeNetwork::admission_gate()` policy getter remains a separate application
API.

The existing optional `EndpointAdmission` policy is now the single internal
discriminant for restricted mode. It still disables TLS early data, restricts
registration and candidate disclosure, and binds an explicit sole custom
contact route to its attempt. Restricted wrappers expose neither ordinary
early-data methods nor conversions to ordinary endpoint/attempt objects.
Their independent admission outcomes, control exchanges, data-path permits,
fresh acknowledgment, confirmation, deadlines, cancellation and revocation
remain in the shared implementation. New checks still precede the old
handshake hook and original `Connection` delivery; its `Reject` closes the
connection before handoff.

The permanent ordinary consumer programs are in
`tests/ordinary_endpoint_consumers/`. `tests/ordinary_endpoint.rs` registers
their unchanged source with Cargo. A native-only Noq development dependency
lets the original required socket methods and public `Transmit` literal run
through real UDP I/O. Only the repository's local package lock row changes;
all complete foreign package rows retain their versions, sources, checksums
and dependencies. Library defaults, dependency upgrades, packaging and
transport algorithms are outside this change.

## Verification

Before production edits, the fixed early-data consumer compiled and ran on
official iroh 1.2.0 with its original lock. Identical source failed on the fork
with the expected return-type and exhaustive-match errors (`E0308`, `E0004`,
and consequent early-handle method errors). After restoration, the same source
compiled and exchanged first-connection early data and accepted resumed 0-RTT,
including the original handshake-hook timing and normal endpoint close.

A second fixed official consumer verifies ordinary `Incoming` and `Accepting`
`IntoFuture`, custom `accept_with`, Router early-data handling, before-connect
and handshake veto/short-circuiting, public struct construction, and actual
send/receive through only the old required socket/sender methods. It compiles
and runs unchanged on both official sources and the fork. The original M92
16-line consumer, SHA-256
`e59e299a02e620408c28b0398b1e3ec8ab451ce9f9111e37af2f68e61bdcc769`,
also compiles unchanged against both libraries.

The final affected runtime selection passes all 36 distinct tests: two ordinary
consumer tests, 19 restricted-endpoint tests, 11 Native admission tests and four
application-control tests. The environment-gated interoperability test is run
explicitly with its existing pinned pre-migration counterpart; its initial
ignored selection is not counted as a pass. All 18 positive/negative compiler
contracts pass, including restricted early-data/conversion rejection and
Native member-link rejection of a control capability.

Both current SDKs are rebuilt and verified against their actual selected
inputs. Four owned-loopback Ergo TLS/WSS and local HTTP-relay scenarios pass:

- Actual waiting request, one approval, durable authorization and admission
  secret, both persistent identities reopened, and member PING/PONG.
- Original member connection improved from relay to RTC under the fixture's
  explicit `PreferDirect` policy, retaining its identity/binding, fresh ACK,
  bilateral readiness and member traffic.
- Disabled/unavailable RTC, cancellation and deadline after unqualified channel
  readiness, and local departure at the pre-release checkpoint. The departing
  Browser withholds business and retires its authority; a peer that has not
  learned the departure is assessed separately under the accepted propagation
  window.
- Browser incoming manual approval, precode use/retry, replica, member streams
  and Browser-to-Browser RTC improvement on the original member connection.

SDK close precedes browser/context disposal, with actual WebSocket closure
observations. Native and relay exit normally; owned containers are absent.
Generated TLS private keys and NSS databases are removed only after those
cleanup checks. No public-facility or physical-device qualification is claimed.

The final source digest is
`f48dcdbcf04cf118184ae076f2e3d9289d369ac81dd0681bb5ef02f376f1f231`.
Artifacts, selected inputs, raw results and the separate review axes are linked
from `docs/evidence/ordinary-endpoint-compatibility-2026-10-19/`.
Private SDK logs, identities and SDP remain local; published SDK results use an
explicit scalar/hash allowlist. Two copied SDK controllers retain historical
descriptive scope strings; their execution guards, explicit environment paths
and artifact hashes identify the actual current/current SDK subjects.

## Retained evidence and build boundaries

Local evidence is under `/tmp/miopunch-m95-20261019/`. It retains the original
fork failure, an ordinary-build driver rejection before Cargo, and the second
consumer's initial fixture compilation errors. Those fixture errors were fixed
before its successful official baseline; the qualified consumer bytes are
unchanged afterward. Failed or unexecuted preparations are not reported as
successful builds.

All builds use the shared repository target, fixed locks, stable profiles and
pre-Cargo source/tool/artifact guards. Cleanup removes only exact reviewed
obsolete test-cache entries or verified duplicate cache executables, preserving
dependency libraries, raw dependency files and immutable pins. One obsolete
intermediate ordinary-consumer test executable is stored losslessly compressed,
with its original hash verified by decompression; official baseline programs,
final tests and all SDK pins remain unchanged. The 249 pre-existing WIP paths
remain outside this delivery.
