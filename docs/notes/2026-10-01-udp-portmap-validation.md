# Explicit UDP gateway mapping without a data relay

MIOPUNCH-39 implements the gateway mapping slice of the
[implementation specification](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189).
It reuses portmapper 0.19.3's NAT-PMP client, ownership and lease service,
iroh's actual UDP transport and original TLS/QUIC, and the existing formal
NativeNetwork admission and member gates.

## Configuration and recorded implementation choices

`ResourceConfig.port_mapping` is local gateway consent, separate from
Observation, Lookup, Signaling and DataRelay. `Disabled` sends no gateway
probes or mapping requests; `NatPmp` enables only UDP NAT-PMP to the local
IPv4 default gateway. Users explicitly authorize this gateway by choosing
NatPmp. PCP and UPnP are disabled in this native slice; their runtime support
is not claimed. The legacy algorithm and slow NAT probability matrices are
unchanged.

The existing native runtime previously built without iroh's portmapper
feature. Enabling the dependency now preserves that non-probing default:
missing/old saved configuration and the fork's default builder both remain
Disabled. The upstream explicit Enabled configuration remains available at
the low-level API; it is outside this ticket's validated native capability.
This choice avoids silently beginning gateway discovery or mapping when an
existing node upgrades. No default public facility or data relay permission
grants local gateway consent.

Configuration is saved, restored and backed up with the existing private
atomic resource setting API. Trusted default-table updates and replacement
imports preserve the local mapping choice. As with MIOPUNCH-38, changes
apply on the next endpoint start, not as a live permission update. Example:

```rust
let mut settings = node.resource_configuration()?;
settings.port_mapping = miopunch_native_probe::resources::PortMapping::NatPmp;
node.save_resource_configuration(&settings)?;
let endpoint = node.bind_configured_endpoint(bind_ip).await?;
```

## Socket, candidates and ownership

The mapped internal port comes from the actual IPv4 iroh QUIC socket. The
existing empty-observation-map flow already procures a mapping before
skipping QAD, so no observation or relay facility is required. A successful
gateway response supplies a candidate; only actual formal admission and
member data exchange prove peer reachability.

The portmapper service renews at half the gateway-granted lease. Successful
renewal preserves or replaces the candidate. Failed renewal cannot extend
it: failure conservatively withdraws it from `Endpoint::addr`, and an
unrenewed lease also expires at its original deadline. Mapping watcher changes
now publish directly, without waiting on QAD or an observation report.
Silent administrative deletion or a restart has no instantaneous notice
contract: the next renewal observes the change, and an unrenewed candidate
is removed no later than the original lease deadline. Copied endpoint addresses remain
hints, not a permanent reachability guarantee.

Graceful `Endpoint::close` waits for the mapping service to cancel pending
creation and attempt release of its own mapping, with a one-second bound.
The portmapper adaptation also retains cleanup responsibility immediately
after an acknowledged NAT-PMP allocation. A subsequent failed or cancelled
public-address query sends a nonblocking deletion for that known owner;
failed renewal withdraws any older advertised candidate. Cancellation waits
for this cleanup guard, and a completed pending mapping is released before
shutdown acknowledgment. It does not
wait for a gateway deletion response or promise cleanup when a gateway is
unreachable; the remaining lease expires there. Abort queues best-effort
deactivation. No gateway mapping table is enumerated or cleared, and no
foreign owner's port is deleted. An unavailable mapping does not prohibit
other allowed direct paths.

## Real validation and limits

The reproducible entry is `scripts/check_udp_portmap.py`. It compiles the
public test worker and runs one owned offline Docker container with isolated
network namespaces. A controlled RFC 6886 NAT-PMP server installs and
removes actual Linux DNAT/FORWARD rules; it does not inject endpoint
candidates or fake successful transport. The remote node has no route to
the node's private LAN and initially reaches it only via the advertised
external mapping. Packet capture ties external and internal socket ports.
The fixture grants a six-second lease for bounded lifecycle checks.

Each case runs separately after the first success and priority failure:

```sh
rtk proxy python -B scripts/check_udp_portmap.py --case positive --output NEW_DIRECTORY
rtk proxy python -B scripts/check_udp_portmap.py --case expired --output NEW_DIRECTORY
rtk proxy python -B scripts/check_udp_portmap.py --case changed --output NEW_DIRECTORY
rtk proxy python -B scripts/check_udp_portmap.py --case restart --output NEW_DIRECTORY
rtk proxy python -B scripts/check_udp_portmap.py --case renewed --output NEW_DIRECTORY
rtk proxy python -B scripts/check_udp_portmap.py --case disabled --output NEW_DIRECTORY
```

Additional `allocation-timeout`, `allocation-cancel` and
`renewal-address-failure` cases exercise acknowledged allocations whose later
address query fails or is cancelled, including candidate withdrawal.

The cases cover formal manual approval, durable authorization and admission
secret delivery, member traffic, failed renewal/withdrawal/fresh failure,
allowed LAN fallback, external port replacement, actual gateway process
restart, two successful renewals beyond the original lease, explicit disable
and ownership-preserving close. The disabled protocol packet check covers
PCP and UPnP, as well as no NAT-PMP access in Disabled mode. Private test
stores are created in the container's temporary directory and deleted;
committed evidence contains only public results, gateway requests, rules and
packet captures. The container, processes and namespaces are owned and
removed by the runner.

See [the evidence index](../evidence/mio39-udp-portmap/README.md) for exact
runs, failures and affected regressions. The lab image is the existing
`miopunch-nat61-lab:bookworm`, built from the committed
`scripts/legacy_nat_multi_socket_lab.Dockerfile`; its Linux network tools
are reused without adopting the old NAT algorithms or test gates. Docker
privilege is limited to the owned offline lab's namespaces and rules.

This is Linux, IPv4 UDP NAT-PMP against one controlled protocol service and
actual kernel forwarding. It does not claim PCP/UPnP, hardware-router or
cross-operator success rates, TCP mapping, browser gateway access, runtime
hot configuration, new membership rules, UI, full TUN/proxy, KCP or custom
congestion control. Browser/WASM checks establish compilation only.
