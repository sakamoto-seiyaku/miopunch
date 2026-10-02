# IRC signaling through the configured native connection entry

MIOPUNCH-45 adds IRC to `NativeNetwork::bind_configured_endpoint[_with]`.
The caller still saves permitted resources, registers member protocols and
uses the ordinary connect/accept/stream interfaces. IRC carries opaque QUIC
datagrams during restricted bootstrap. The existing TLS target verification,
AdmissionGate proof and shared `message_bootstrap` coordinator precede ordinary
data release. The returned Connection has the same stable ID as its protected
bootstrap, and ordinary business uses a validated UDP path.

This implements the [ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/54089142-f52c-42f2-9643-38cdeaf2ccdd)
at the parent specification's already approved node, configuration,
Endpoint/Connection/stream and message-adapter boundaries. The prior
[MQTT qualification](2026-10-02-mqtt-bootstrap-validation.md) and
[member signaling qualification](2026-10-02-member-signaling-validation.md)
remain applicable. Neither carrier visibility nor server TLS is membership
authority or the end-to-end protection mechanism.

## Recorded adapter choices

The supported descriptor is `irc://HOST:PORT/` with an explicit nonzero port
and `Signaling` permission. Credentials, query options and alternate channel
paths are rejected. No default public service is added. Each configured origin
has a domain-separated public 16-byte tag and a random 16-byte delivery token,
forming a 32-byte custom address under IRC's distinct transport ID. Do not
truncate this to the MQTT legacy 16-byte route or parse it as a member route.

The adapter registers a random temporary nickname and joins `#miopunch-v1`.
It advertises only after registration, ISUPPORT and successful channel join;
numeric service refusal, kick, TCP interruption and endpoint shutdown withdraw
the advertisement. Origins remain independently qualified: an unavailable
origin does not prevent another permitted one from working. Saved signaling
disables apply to the next endpoint and actual carrier use.

`MIR1` channel envelopes contain the target and source tokens, a random 64-bit
fragment-group identifier, an index/count, and hex-encoded opaque bytes. This
identifier groups fragments, not identities, permissions or connection attempts.
The existing QUIC engine owns each authenticated attempt and its stable ID.
Spoofed public tokens do not authenticate a packet or bypass the TLS/member gate.
Invitations, membership proofs, admission secrets and private candidates are
not added to the public envelope.

Each datagram is at most 2,048 bytes and splits into at most 16 chunks of 128
bytes. Outbound lines are at most 512 bytes including CRLF; the qualified
server's delivered prefixes also fit that traditional bound. Input lines are
bounded before parsing. The wire choice follows the traditional
[IRCv3 message-size limit](https://ircv3.net/specs/extensions/message-tags.html)
and its [prefix accounting guidance](https://ircv3.net/specs/extensions/multiline).
It does not require IRCv3 multiline support.

Incoming/outgoing packet queues have depth 64. Reassembly retains at most 32
groups, each at most 16 chunks, with a fixed three-second lifetime. Duplicates
do not extend that lifetime; inconsistent fragments are rejected. Completed
group suppression is bounded to 128 entries and three seconds. QUIC remains
responsible for duplicate packets and retransmission. These are implementation
safety bounds, not capacity or latency promises.

Sending is paced to one fragment per ten milliseconds. Queued datagrams older
than 500 milliseconds are discarded before sending; a write stalls for at most
two seconds. The real wrong-target test exposed stale fragmented handshake
backlog starving a subsequent legitimate attempt. Its public-interface RED
became GREEN after adding queue expiry. The service must support the tested
rate and datagram lifetime; the adapter does not defeat server flood protection.

Reader and writer futures share one owned driver, so partial line reads are not
canceled by outbound events. Endpoint shutdown drops both TCP halves and queues.
Disconnection fails the original attempt; there is no transparent reconnect of
an active QUIC session. A fresh configured bind reconnects with new nickname
and delivery token while retaining the durable node identity and authorization.
An ordinary server accept loop continues after rejected incoming handshakes,
as required by the existing iroh API.

## Actual qualification and limits

The reproducible entry is `scripts/check_irc_bootstrap.py --output <new-dir>`;
`--short` adds affected regressions and compile/format checks. The pinned real
Ergo 2.7.0 image and checked-in server configuration are recorded in
`scripts/irc_lab.Dockerfile`, `tests/fixtures/irc/ergo.yaml` and the
[evidence index](../evidence/mio45-irc/README.md).

Qualification uses Linux loopback, independent persisted native members with
real manual approval and durable admission materials, real IRCd and real
QUIC/TLS. The first connection knows only the target's IRC route, not its UDP
address. The coordinator then validates UDP on that original session. The
priority check drops the final IRC fragment of real multi-line QUIC handshakes,
returning bounded unavailability without a business connection. Further checks
cover reassembly limits, repeated fragments, independent channel observation,
malformed injection, wrong TLS target, another unapproved invitation holder,
member retry, cancellation, interruption and resource disables.

The usable fixture disables server fakelag and history. A second real IRCd
enables its actual five-command burst/two-messages-per-second fakelag; it does
not qualify for this adapter's bootstrap rate and returns bounded unavailability.
These settings are explicit capability limits. IRC's ordinary low flood limits,
authenticated services, bouncers, history replay, larger line extensions,
`ircs` server TLS and arbitrary public IRC networks are not qualified here.
The plaintext server listener is published only on host loopback; inner
end-to-end protection still comes from TLS/QUIC. Public channel participants
can observe and disrupt routing metadata, volume and timing. Marker observations
over decoded opaque chunks are not a cryptographic audit.

An owned offline container cuts all of its IPv4/IPv6 UDP paths after actual
business succeeds, while leaving IRC TCP running. Queued business fails on the
original connection and independent IRC publication counts stay unchanged.
No application retry masquerades as the original connection. This is a real
kernel path-loss result, not a new NAT success claim. Only UDP data release is
qualified by this declared environment; broader native-relay, NAT, platform,
browser and recovery support remain outside this increment. No custom congestion
controller, KCP, UI, service ACL or general runtime plugin platform was added.
