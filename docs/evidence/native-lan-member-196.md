# MioCube offline LAN member connections

MIOPUNCH-196 is qualified on Linux using two independently reopened native
clients in separate network namespaces. Both clients were admitted before cold
startup. The runtime namespaces share only a veth LAN, have no WAN/default
route, and start without `member.contacts.json`. The caller supplies the member
identity and an allowed, unreachable public relay. No LAN address is supplied
to the caller.

The entry is `NativeClient::open` followed by `NativeClient::ping`. Both ends
read PONG twice over the same `MemberConnection` stable ID. Each end verifies
the authenticated peer and a selected IP path. The IPv6 case verifies that the
selected fe80 address uses the receiving client's own interface index. The two
interfaces deliberately have different indices.

## Results

The final run is `/tmp/miopunch-196-runtime-final` and the immutable executable
is `.artifacts/miopunch-196-lan-final.test-bin`. Its build receipt is
`/tmp/miopunch-196-final-native/receipt.json`, with 462 Fresh artifacts and
7 rebuilt artifacts. The later compatibility correction preserves this explicit-interface branch and
restores default-interface sending for unchanged library consumers. Native
checking passed at `/tmp/miopunch-196-default-compat-check/receipt.json`.

| LAN | Client | Server | Selected paths |
| --- | --- | --- | --- |
| IPv4 | 2.067 s | 2.650 s | 10.196.0.1 / 10.196.0.2 |
| fe80 IPv6 only | 1.692 s | 2.252 s | fe80::196:1%4 / fe80::196:2%5 |

Both cases returned exit 0. Per-process logs, interface addresses and routing
tables are retained in the runtime directory. The public relay attempts were
still pending when LAN admission and business completed, and were cancelled
with the losing dial lanes.

## Reproduction

Build `tests/lan_member.rs` with `nat-lab-test-hooks` through the shared Cargo
cache, then pin the executable reported by Cargo. Run the script in a disposable
user, mount and network namespace.

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/lan-member-build -- \
  test --locked --offline --features nat-lab-test-hooks --test lan_member --no-run --jobs 2
rtk proxy python3 scripts/pin_artifact.py CARGO_EXECUTABLE .artifacts/lan-member.test-bin
rtk proxy unshare -Urnm rtk proxy bash scripts/check_native_lan.sh \
  /ABSOLUTE/REPO/.artifacts/lan-member.test-bin /tmp/lan-member-runtime
```

The script mounts its temporary `/run` within the disposable mount namespace.
It creates and removes its own network namespaces. Qualification covers the
Linux native client and local multicast/unicast behavior. Physical devices and
other operating systems were not exercised in this run.

## Causes addressed

- Fusion address discovery now runs alongside public contact attempts, and a
  slow discovered address cannot prevent another discovered candidate from
  starting.
- The pinned swarm discovery implementation keeps its wildcard mDNS receiver
  when an offline host lacks a default multicast route. It joins and sends on
  the actual local interfaces.
- mDNS adds usable link-local addresses from the actual IPv6 listener, and
  discovered fe80 candidates retain the receiver's interface scope. Ordinary
  iroh address publication remains unchanged.
- IP normalization preserves IPv6 scope through iroh's address conversion,
  receive metadata and final UDP sender.
- Final member admission completes an initially unknown wildcard source IP from
  the same live QUIC path. Remote address, socket ownership and exact binding
  qualification remain enforced.

## Regression and review

The selected native library, resources, member contacts and unified-connection
suites were built once at `/tmp/miopunch-196-regression-build/receipt.json`
(463 Fresh, 9 rebuilt). The full selected set recorded 132 passed, 9 failed and
17 ignored. Results and logs are in `/tmp/miopunch-196-regression-runtime`.
The two enhanced-UDP failures passed unchanged when run individually from the
same immutable current executable, with the original assertions retained.
Those results are in `/tmp/miopunch-196-enhanced-focused/results.json`.

The remaining seven failures also reproduce on baseline `c1a9a3a8`, under the
same host permissions and serial execution. Baseline comparison used the same
source paths and shared Cargo cache, restored the six explicit source/manifest
files before runtime, and preserved the user documents and compatibility patch.
Its results, logs and build receipt are in `/tmp/miopunch-196-baseline`.
The reproduced failures concern legacy coordinator path snapshots and warmup,
legacy fallback error classification, interrupted enhanced observation, and
p2panda high-level publication. These checks are independently obstructed on
this host, so this evidence does not claim a green selected regression set.

The first independent Standards/Spec review found one default-consumer
compatibility issue and zero Spec issues. The default-consumer issue was
corrected, typechecked, and resolved by focused read-only Standards review.
No further review finding remains.
