# Formal native TCP direct paths

MIOPUNCH-40 integrates the [MIOPUNCH-18 carrier](2026-09-29-tcp-carrier-validation.md)
with the formal native node, shared admission gate and
[startup resource policy](2026-10-01-resource-policy-validation.md).
The implementation uses the same durable Ed25519 node identity, iroh TLS/QUIC,
SCRAM admission proof, signed management records and SQLite consumer as UDP
and allowed relay connections. TCP reachability grants no membership.

## Recorded startup configuration

`NativeNetwork::save_resource_configuration` persists these additional fields
in the existing private, atomic `resources.json` configuration:

```json
{
  "tcp_listen": "127.0.0.1:0",
  "udp_direct_disabled": true,
  "tcp_direct_disabled": false,
  "disabled_tcp_addresses": []
}
```

This is an excerpt, not a complete configuration file. Set these fields on the
existing `ResourceConfig`; other fields retain their existing semantics.
`tcp_listen: null` preserves the previous absence of an automatically installed
TCP listener. Port zero asks the OS for a port; a nonzero port binds explicitly.
The node installs this carrier through its normal and advanced builder paths.
An advanced caller which installs its own TCP factory can leave `tcp_listen`
unset. `udp_direct_disabled` removes all native UDP data sockets at startup.
`tcp_direct_disabled` disables TCP globally, even if an advanced builder adds
a carrier. `disabled_tcp_addresses` denies exact literal IP/port candidates.
These restrictions are independent of Observation, Lookup, Signaling,
DataRelay and local gateway mapping consent. A direct peer is not an auxiliary
relay facility. Updates take effect on the next endpoint start, as in ticket38.

Local disables survive trusted replacement imports and signed resource-table
updates because those operations retain the existing local configuration.
Old configuration files omit these fields and deserialize with their defaults.
TCP listen/disables are not a service ACL or a new member role.

## Candidate and connection interfaces

`ResourceEndpoint::addr()` adds qualified TCP custom addresses to the ordinary
UDP/relay contact address. `TcpTransport::address(SocketAddr)` encodes an
explicit reachable TCP address for an invitation contact, address book or
lookup result; `TcpTransport::socket_addr` decodes it. Contacts continue through
`ResourceEndpoint::connect` and ordinary `Connection` streams.

The fork exposes `Endpoint::custom_addrs()` explicitly. It does not automatically
publish custom addresses through address lookup. The formal resource endpoint
advertises only the recognized TCP data carrier, so signaling-only custom
routes do not become advertised business candidates. A wildcard listener's
address still requires an actual reachable interface IP when supplied to a peer;
its port alone is not proof of reachability. The route preface uses the connected
socket's local IP and the listener port, so wildcard binding does not send an
unspecified IP as the return route.

The existing fork resource policy now qualifies custom peer addresses at
`CustomSender::is_valid_send_addr` and again immediately before `poll_send`.
Every custom carrier is wrapped, including direct carriers without an auxiliary
facility descriptor. Explicit hints, lookup results and raw Endpoint calls
therefore cannot turn a denied TCP candidate into an actual socket dial.
Auxiliary carrier facility/use checks remain independent. The admission gate
still classifies every ALPN and admits TCP using its existing data-carrier ID.
No TCP-specific membership or synchronization protocol was introduced.

## Wire framing, IPv6 and lifecycle

The TCP transport retains ticket18's six-byte IPv4 route preface and packet
framing. Unscoped IPv6 uses sixteen address bytes plus two port bytes in the
same carrier ID. Each listener handles its own address family. Scoped/link-local
and IPv4-mapped IPv6 address modes are rejected/unclaimed; the evidence covers
real IPv4 and IPv6 loopback sockets on Linux, not cross-operator IPv6 reachability.
The preface is an untrusted return-route hint; inner iroh TLS checks the expected
peer identity. Packet lengths remain 1–2048 bytes, with bounded queues and
64 circuit limits. Existing reads/writes, backpressure and endpoint cancellation
retain their bounded behavior.

TCP-configured startup selects the already available noq-proto1.3.0 BBRv3,
with ticket18's five-second connection keepalive and ten-second connection idle
configuration. Path selection stays in iroh's existing quality selector; TCP
is not forced ahead of other allowed paths. QUIC timing retains its original
PTO rules. No congestion implementation or algorithm was added. `TCP_TRAFFIC`
reports payload counts and outstanding/peak queue observations when the carrier
is dropped; this is a snapshot, not a claim that asynchronous task cleanup has
already completed. Capacity remains the existing channel depth32, with a sender
reservation/current packet accounting for peaks above32.

The normal `network run` CLI starts a TCP-only saved configuration and prints its
actual TCP address in `READY`. SIGINT closes the endpoint and node. Controlled
physical loss of the only TCP path ends the old stream and connection; later
contact gets a distinct connection ID. The first cut fixture left a reverse
TCP return route available, and public QUIC statistics showed continuing valid
packets. The corrected fixture constrains both directions to one owned proxy,
then removes both sockets and its listener. This is an experiment correction,
not a requirement to disable working alternate paths in the product.

## Reproduction and scope

The public test seams were already approved in the parent specification and
this ticket: formal node startup/admission, durable reopen/control sync, normal
Connection streams, real socket access and cancellation. Run:

```sh
rtk proxy python3 -B scripts/check_tcp_direct.py --output NEW_DIRECTORY
```

Use `--case TEST_NAME` for one behavior. The runner saves build/test logs and
actual command exits, and uses Cargo's reported executable. The real first
check, immediate identity failure, incremental RED/GREENs, fixture diagnosis and
affected checks are indexed in [the evidence](../evidence/mio40-tcp-direct/README.md).
Cargo also builds its package binaries when compiling an integration test.
On a Linux workspace with little free disk, `--strip-debug` omits the test
executable's debug sections at link time; it does not change test behavior.

This ticket adds explicit reachable native TCP paths, not TCP gateway mapping
or TCP punching. It makes no new NAT success-rate denominator, public facility
availability, other-platform runtime, UI, full TUN/proxy, service ACL, KCP,
Brutal or custom-congestion claim. Browser evidence is compilation only.
