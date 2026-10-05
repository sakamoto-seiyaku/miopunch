# Common connection coordinator: first native member slice

[MIOPUNCH-70](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/06ed42b0-64e0-4d5d-a987-d548715665cf)
implements the first tracer in [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md).
The scope is controlled Native↔Native over existing permitted UDP, with the
original membership and admission-secret gate. RTC, enhanced punching,
relay-first optimization, browser directions and public NAT qualification
remain separate slices. The preceding prefactor extracted existing TCP
retention/recheck from `MemberPathBootstrap` into the common release seam; the
existing persisted native manual-member story passed before introducing wire
changes.

## SDK and ownership

The formal `NativeNetwork::bind_configured_endpoint` assembly returns a
`ResourceEndpoint`. Its `coordinator()` provides:

```rust
connect(target: EndpointAddr, purpose: SessionPurpose, attempt: AttemptContext)
    -> Result<iroh::endpoint::Connection, ConnectError>
accept(incoming: Incoming, purpose: SessionPurpose, attempt: AttemptContext)
    -> Result<iroh::endpoint::Connection, ConnectError>
```

`EndpointAddr` carries the expected identity and contact hints, not grants.
The caller supplies no platform, role, transport or path preference.
`SessionPurpose::MemberBusiness` is the first migrated purpose and uses the
existing `ProtocolScope::Member`; additional purposes are not silently routed
through it. The returned original iroh Connection keeps `open_bi`, `accept_bi`
and ordinary stream semantics. Native persistent identity and consent remain
owned by the NativeNetwork assembly.

`AttemptContext` contains one absolute deadline and cancellation token.
Outgoing attempts pass both to the existing iroh options; incoming attempts
pass the original deadline and are cancelled by dropping the protected
acceptance future. The single outer budget covers TLS, proof, capability
exchange, probe and bilateral readiness. Incoming operations get no new fixed
product timeout. This slice has no background exploration, maintenance lease,
quality timer or retry loop. Existing Node maintenance and iroh selection keep
their ownership.

## Release phases and strict wire

Planning checks the startup UDP consent and removes non-UDP contact hints from
this tracer. No permitted candidate returns `DataPathUnavailable` without
creating an ordinary handle. Restricted is the existing iroh TLS handshake,
AdmissionGate secret proof and membership stage. A shared endpoint hook then
exchanges the new protected capability envelope on the original M. Probing pins
its exact selected IP path and waits for a newly started authenticated probe's
ACK. Committing exchanges nonce-bound readiness on the same protected stream,
requires stream EOF, and rechecks the original gate and qualification revision.
It then re-reads the exact live path ID/address and its existing probe's live
acknowledgement before release. A path retired during readiness cannot borrow
another private control route's survival. Selection may change while that
qualified path stays open; release does not impose a new selector rule.
Active returns the ordinary original M. Failure, cancellation or deadline closes
that pending M using the existing iroh pending-admission guard and control guard;
another live M remains independent. Successful setup disarms the guard, so its
expired setup budget does not become a session lifetime.

The new entry uses only `miopunch/session/member/1`. It has no alternate ALPN or
legacy capability fallback. One u16 big-endian length prefix must equal 28;
larger claims are rejected before allocation. Its fixed envelope is:

| Bytes | Meaning |
| --- | --- |
| 0–3 | `MUC1` magic |
| 4 | strict version 1 |
| 5 | purpose 1: existing member business |
| 6 | native runtime 1 |
| 7 | this attempt's initiator 1 or responder 2, derived from QUIC side |
| 8 | runtime UDP capability mask, bit 0 |
| 9 | effective local UDP permission mask, bit 0 or zero |
| 10–11 | reserved zero |
| 12–27 | fresh per-side attempt nonce, protected and never logged |

Runtime support and resource permission are separate. Unknown bits, reserved
values, wrong purpose/runtime, contradictory roles and malformed lengths are
invalid; unknown magic/version is unsupported. A fixed mask has one position
per capability, so duplicate entries cannot be encoded. The effective bilateral
permission intersection must contain UDP. The 17-byte readiness response is
`1` plus the other side's nonce. Trailing/duplicate readiness data and a wrong
nonce are refused. This stream identifies one M; no peer-wide bootstrap cache,
cross-session lease or exporter logging is introduced.

The stable SDK error classes are `IdentityOrAdmissionRejected`,
`ProtocolUnsupported`, `InvalidControl`, `DataPathUnavailable`,
`DeadlineExceeded`, `Cancelled` and transport errors. The existing iroh hook
error string is mapped at this boundary; the fork need not learn product wire
semantics. An independently terminating peer may surface as a transport close.

## Fork inventory and remaining callers

This ticket changes no iroh/noq source. It reuses `EndpointHooks::before_data_release`,
`ConnectOptions` caller deadline/cancellation, `Incoming::with_caller_deadline`,
private control streams, `Path::start_probe`/fresh authenticated ACK and original
pending-attempt cleanup. The configured assembly advertises and gates the new
member ALPN; the old bootstrap hook delegates direct retention to the shared
release seam and yields the new ALPN to the coordinator.

Existing internal old-protocol callers remain available for the later migration
slices, including ongoing narrow STUN/IRC investigation. They are not a wire
compatibility promise of the new coordinator. The final contract slice removes
remaining migrated old callers and wire branches.

## Executable evidence

`tests/unified_connection.rs` uses public NativeNetwork admission operations to
independently save two identities and both member materials, closes both nodes,
and reopens them. The success test invokes formal coordinator connect/accept,
checks both original gates and identities, and transfers exact request and
response contents on an actual QUIC bi-stream. Adversarial member hooks mutate
only the real protected wire; they do not mock TLS, proof, streams or path ACK.
Wrong version, illegal capability, contradictory role and oversize all refuse
release while an independent healthy stream continues. Pending cancellation and
deadline close the exact original M. Denied UDP and signaling-only hints do not
produce a normal handle.

The concentrated Standards and Spec review each identified the same P2: the
historical native probe could survive in a snapshot across the readiness await.
The [red receipt](../evidence/unified-native-2026-10-05/review-red/receipt.json)
reproduces actual premature ordinary release: UDP path 0 was freshly ACKed,
then retired while real TCP still carried restricted private control on the same
M, and valid late readiness returned a normal SDK handle. The added regression
pins this sequence with existing endpoint/path APIs. The live path/probe
recheck now refuses `DataPathUnavailable` and closes only the pending M.

The first sandbox EPERM and first pending-control fixture timeout are retained
in the receipt. The latter reused the healthy endpoint's peer route cache; a
fresh pending dialer isolated the controlled route without changing product
deadlines or retry behavior.

Run the SDK artifact and one complete affected regression batch with:

```sh
rtk proxy python3 scripts/check_unified_native.py --output docs/evidence/unified-native-2026-10-05
```

The [initial receipt](../evidence/unified-native-2026-10-05/receipt.json) preserves
the frozen implementation and complete 77-test affected legacy regression.
After the concentrated review fix, only the changed new SDK area is rerun:

```sh
rtk proxy python3 scripts/check_unified_native.py --output docs/evidence/unified-native-2026-10-05/post-review --prior-regression docs/evidence/unified-native-2026-10-05/receipt.json
```

The [final receipt](../evidence/unified-native-2026-10-05/post-review/receipt.json)
records exact
commands and exits, enumerated source hashes, compiled artifact hash, roles,
actual routes, identity-match booleans, fresh probe acknowledgements, caller and
independent cleanup timings, and actual socket rebind observations immediately
after graceful close and 100ms later. Raw stdout/stderr accompany each command.
The normal tracer owns no RTC PeerConnections or custom pumps/queues. The race
fixture additionally owns real TCP control listeners and adapter queues; their
actual rebinds and zero outstanding queue counters are checked after graceful
close. No public listeners, forwarders or external temporary processes are
needed. Logs omit identity
material, private keys, admission secrets, raw SDP, nonces and TLS exporters.
