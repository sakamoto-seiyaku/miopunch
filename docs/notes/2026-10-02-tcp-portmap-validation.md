# TCP gateway mapping for formal native nodes

MIOPUNCH-41 extends the formal TCP carrier from
[40](2026-10-01-tcp-direct-validation.md) with the NAT-PMP lease and cleanup
service already used for [UDP mapping](2026-10-01-udp-portmap-validation.md).
The [parent specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)
and ticket41 define the behavior; this note records the concrete integration.

## Configuration and listener ownership

The existing local `ResourceConfig.port_mapping = PortMapping::NatPmp` consent
also enables an independent TCP NAT-PMP client when `tcp_listen` is configured
and direct TCP is allowed. No relay, observation, lookup or signaling permission
enables gateway access. Disabled remains the default for old configurations.
Settings take effect on next endpoint startup, with the same persistence,
backup and trusted-update behavior as tickets38–40.

The TCP service binds its lease to the actual OS listener port and requests
NAT-PMP TCP (opcode2), never reusing the UDP socket's external address or lease.
TCP and UDP can use identical numeric private/public ports but remain separate
protocol leases. IPv4 wildcard listeners and listeners bound to the default
gateway's local IPv4 address are eligible. A listener bound only to a different
interface cannot receive that gateway mapping and does not create one. IPv6
TCP remains a direct carrier; IPv6 gateway mapping is outside this capability.
Only NAT-PMP is enabled; PCP and UPnP remain disabled and unclaimed.

For advanced startup, `TcpTransport::with_nat_pmp_mapping()` requests the same
optional capability. The fork passes the endpoint's actual gateway consent and
custom address policy into binding, so a saved mapping disable or global TCP
disable also suppresses requests from an explicitly installed TCP factory.
A standalone factory's explicit bind is outside a configured endpoint and
requires its own explicit mapping opt-in.

## Candidates, circuits and authentication

The TCP endpoint publishes its independent mapped address through its existing
custom address watcher. `ResourceEndpoint::addr()` exposes it through the same
qualified TCP candidate API; contacts use `ResourceEndpoint::connect` and normal
`Connection` streams. Allocation success supplies a candidate, not proof that a
peer is reachable or a member. The controlled remote test has no route to the
private LAN and must use actual kernel TCP DNAT before formal admission works.

The retained IPv4/IPv6 preface remains an untrusted return-route hint. A mapped
route can differ from a server's private listener IP and port. For an outbound
circuit the carrier now keeps the dialed route as its routing key instead of
requiring preface equality; responses stay on the established circuit. The
preface cannot replace the expected iroh TLS endpoint identity. Framing, bounded
queues, circuit limits and the existing BBRv3 configuration are unchanged.

Portmapper's half-lease renewal updates or withdraws the custom candidate.
Failed renewal and a subsequent failed/cancelled address query cannot preserve
a candidate for a mapping that was released. Existing TCP circuits and fresh
dials have independent outcomes: deleting/replacing a forwarding rule can leave
an established conntrack circuit usable while new connections to the removed
port fail. The tests transfer again on that same authenticated connection and
separately try fresh old/new routes; no connection is silently recreated.
Administrative deletion has no instant-notification contract: the service
observes renewal results and the bounded lease deadline, as in ticket39.

## Cleanup and recorded source correction

The fork retains an optional cleanup handler independently from Noq's custom
receiver. Raw `Endpoint::close()` drains QUIC, then bounds each handler to one
second before actor cancellation. TCP deactivates its own portmapper service,
which waits for pending allocation cleanup and releases known active ownership.
Abrupt receiver drop makes a bounded best-effort attempt; an unreachable gateway
or process loss still relies on lease expiry.

The real priority failure showed that upstream's completed NAT-PMP release
always encoded UDP. `Mapping` now retains its allocated protocol and releases
that protocol, as its pending-allocation guard already did. An actual UDP socket
and same-IP/same-port UDP lease continue exchanging data after TCP closes; foreign
TCP and UDP rules remain intact. No foreign mappings are enumerated or deleted.

## Reproduction and evidence limits

Run one behavior at a time in a new output directory:

```sh
rtk proxy python3 -B scripts/check_tcp_portmap.py --case positive --output NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_portmap.py --case close-isolation --output NEW_DIRECTORY
```

The runner compiles the public worker, uses Cargo's reported executable and
starts one owned offline Docker container on the existing Linux lab image.
Its NAT-PMP wire service installs real kernel forwarding rules; it does not
inject candidates, fake TLS, simulate stores or report reachability without data.
Routine builds use the project shared `.cargo/config.toml` dev/test defaults.
`--strip-debug` is an optional override for a disk-constrained Linux build,
reducing only this native package's debug output and incremental cache. `--worker PATH` can reuse a completed
build while sources are unchanged; command receipts name it. Private stores
and keys remain in container temporary directories and are removed with the
owned processes, namespaces and container.

The approved seams are formal node startup/admission, public resource candidates
and connections, actual sockets and gateway leases. See
[the evidence index](../evidence/mio41-tcp-portmap/README.md) for real RED/GREENs,
fixture corrections and affected short regressions. This is Linux IPv4 NAT-PMP
against a controlled gateway, not hardware-router, Windows, browser gateway,
cross-operator or NAT probability evidence. Previously verified unchanged slow
NAT matrices are omitted under the user's standing policy. No UI, service ACL,
full TUN/proxy, TCP punching, KCP or congestion implementation is added.
