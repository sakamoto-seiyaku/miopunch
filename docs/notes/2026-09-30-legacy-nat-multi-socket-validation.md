# Legacy mode 2/4 physical listener validation

MIOPUNCH-61 restores the old physical multi-listener execution inside one
iroh Endpoint and the original authenticated QUIC connection. A bounded
socket pool gives each real descriptor a local socket ID. Receive metadata,
authenticated nonce correlation, private mapped local tokens, explicit QUIC
path validation, source selection and path lifetime all preserve that ID.
The token is resolved to the real interface IP and descriptor before sending.

The implementation keeps the old 256 bind attempts, 1000 random iterations,
1024–65534 port distribution, three TTL indexes, 3000 ms sender delay, mode 4
inclusive range-2 formula, and 38000 ms absolute phase budget. Individual bind
and ordinary destination-send failures are reported and continued. Scope,
deadline, cancellation and resource limits terminate the round explicitly.
Provisional winners close on cancellation; committed winners follow the
original connection and final Noq path disposal. Shared primary IPv4/IPv6
descriptors remain Endpoint-owned.

The controlled fixed matrix produced 18/30 normal-positive business successes
and 12/30 complete misses. Auxiliary cells produced one business success,
three misses and four admission rejections. All 19 successes used a genuine
nonprimary descriptor in at least one peer's original connection. All 38
classification/packet gates passed and 16 corrupted evidence variants failed.
See the [retained evidence](../evidence/mio61-legacy-multi-socket/README.md) for
topology ratios, costs, generation separation, observed failures and limits.

Two runtime adaptations were necessary. The peers now exchange protected
winner readiness and validate the exact winner path within the remaining
original deadline, instead of imposing a new 3 s handoff window. Periodic QAD
reports keep their aggregate candidate generation but do not advance the
physical network epoch used by an already acquired sequence or fallback
snapshot. Actual network changes still invalidate pending work. No old
strategy detail, timeout extension, alternative dial or MQTT data fallback
was substituted.

The final serial real-MQTT workspace suite passed 236 tests with zero failures
and three existing ignored tests. Targeted source/TTL/descriptor and handoff
checks and native/WASM builds passed. This is Linux physical UDP evidence;
other platforms and uncontrolled NAT success rates remain unmeasured.
The matrix did not collect per-process CPU usage; it records that missing
required cost metric explicitly and does not infer CPU from elapsed time.
A separately frozen three-process functional supplement now records actual
whole two-node debug-process CPU: mode 0 baseline 0.527418 s user / 0.058140 s
system, full mode 2 13.649365 / 5.111731 s, and full mode 4 2.816101 / 0.629761 s.
Real TLS/QAD/control and physical sockets execute the complete 1000/256/38 s
work behind raw-only loopback INPUT loss. Exact counters, original control
PING/PONG and FD cleanup passed. These include setup and diagnostic costs,
add no NAT samples and do not reconstruct the historical cells' CPU.
Automatic mode selection is the next ticket, MIOPUNCH-60, rather than a result
already delivered by this explicit legacy adapter.
