# Formal UDP/TCP STUN integration

Specification: [MIOPUNCH-67](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/058e5f38-8632-4dba-92af-f1f28bbae3d5), parent MIOPUNCH-14.
Accepted policy and protocol sources: [address observation policy](../../notes/2026-10-03-address-observation-policy.md).
The separately saved [fake TCP research](../../notes/2026-10-03-fake-tcp-principle-research.md) belongs to low-priority MIOPUNCH-66; no fake TCP work is included.

## Real checks

The public seams are the already approved configured native node, resource
configuration, Endpoint/Connection/streams, durable membership/replica,
identity, cancellation and actual sockets. The independent fixture services
speak QAD/TLS and RFC8489 Binding; STUN replies reflect actual kernel tuples.
No NAT label/mode or candidate override enters the formal runtime.

`formal/` retains the actual combined run starting with QAD success. Both
nodes are real formal NativeNetwork instances behind two Linux kernel
MASQUERADE gateways, using ports 42027/43027 and MQTT solely for protected
control. The fixture explicitly permits Observation, Signaling and UDP data,
with TCP and data relay disabled for this UDP slice.

| Ordered phase | Observed outcome |
| --- | --- |
| QAD | Two actual destinations `100.64.27.1:7842` / `100.64.27.5:7842`; valid HTTPS sequence; existing mode 0/index 0; zero STUN requests; new signed record saved 2 → 3; original Connection business |
| QAD-only outage | Only WAN QAD input is dropped. Four ordered STUN destinations across two IPs/two ports use the same primary UDP ports; Easy features and existing mode 0/index 0; record 3 → 4; original Connection business |
| STUN disabled | Same saved nodes and resource table; QAD still unavailable; all four STUN Observation origins explicitly disabled. No STUN packets/samples or business handle; bounded failure (client 10106 ms); no leftover UDP or SYN-SENT sockets after owned workers exit |
| Exact restore | Original configuration restored and read back; QAD remains unavailable; fresh STUN sequence; same durable identities/membership; record 4 → 5 and new Connection business |

Initial background QAD responses are withheld until both formal nodes reach
their ordinary observation round, so the direct-first path cannot preempt
the QAD sequence check. The healthy QAD service is then released. Other UDP
phases change only the stated facility rule/configuration. This is a controlled
functional fixture, not arbitrary NAT or public success-rate evidence.

The TCP part uses a configured `stun+tcp://stun.miopunch.test:3478/` origin,
real endpoint DNS and retained same-listener-port Binding streams. Both
Linux IPv4 nodes perform simultaneous open through the two kernel NATs,
validate a TCP path on their original TLS Connection, retire MQTT, save an
actual new signed record 2 → 3 and exchange authorized business. Literal TCP,
one-sided observer failure with permitted UDP fallback, and mixed TCP/UDP
capabilities have separate actual regression receipts in `compatibility/`.

`android/` is a narrow real Pixel 6a / Android 16 API36 foreground JNI
regression of the shared TCP observation helper. Public
`turn.cloudflare.com` resolved to `141.101.90.1:3478`; actual query ports
matched each registered listener in both physical runs. First and restored observations
validated IPv4/IPv6 mapped responses; Observation disable selected no query;
identity remained unchanged and the owned phone directory was removed.
This does not claim Android TCP punching, a new Android application or public
UDP traversal. MIOPUNCH-64's previous public UDP timeouts remain unqualified.

The UDP sequence regression verifies one data binding, actual distinct
destinations/order, cancellation/supersession invalidating old snapshot copies,
fresh replacement and endpoint close. A second real-protocol regression
rejects two DNS/literal names that resolve to the same actual destination.
`coalesced/` additionally maps two healthy QAD service names to one actual
destination. The invalid QAD classification input selects a fresh distinct
STUN sequence, then completes original-Connection sync 2 → 3 and business.
Multiple DNS names therefore cannot manufacture multi-destination evidence
in either protocol.

## Reproduction

Existing `miopunch-nat61-lab:bookworm` is the owned disposable Linux fixture
image; containers use `--network none`, a read-only repository mount and
fresh output/store directories. No foreign container, namespace or device
network setting is changed.

```sh
rtk proxy env CARGO_INCREMENTAL=0 python3 -B scripts/check_stun_flow.py --case both --output NEW_DIRECTORY
rtk proxy env CARGO_INCREMENTAL=0 python3 -B scripts/check_stun_flow.py --case coalesced --output ANOTHER_NEW_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_punch.py --case positive --output NEW_LITERAL_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_punch.py --case observer_failure --output NEW_FAILURE_DIRECTORY
rtk proxy python3 -B scripts/check_tcp_punch.py --case mixed_tcp --output NEW_MIXED_DIRECTORY
```

Build the actual Android node/library/JNI shell with
`scripts/build_android_node.py --jni-shell`, then run:

```sh
rtk proxy python3 -B scripts/check_stun_android_observation.py --adb ADB_PATH --binary ANDROID_NODE --library ANDROID_LIBRARY --shell ANDROID_SHELL_JAR --output NEW_ANDROID_DIRECTORY
```

The physical runner uses only newly created stores and startup configuration
inputs, reads configuration back through the public node CLI, checks actual
observation results and removes only its recorded owned processes/directory.
It sends no invitations, credentials, membership or business data to STUN.

## Affected validation and original failures

Nine affected short Rust targets report 132 passed / 0 failed / 1 existing
ignored (`checks/affected-tests.log`). The first batch's broker-dependent
early returns are not MQTT execution evidence. The separate two-real-broker
follow-up runs MQTT/bootstrap and public resources (18 harness passes), filling
those checks; one MQTT worker entry is inert without its external worker role.
The unique union therefore has 131 executed tests plus that inert entry.
The real broker receipts and owned-volume cleanup are retained.

Native workspace/all-target checking with test-hook features, browser WASM
checking, Android binary/library/JNI build, root and fork formatting and
diff-check all passed. No unchanged slow NAT probability/multi-socket matrix
was repeated. Existing compiler warnings are recorded, not new failures.

`failures/` retains genuine pre-integration TCP domain and UDP formal REDs:
no mutual TCP data capability; respectively no mapped/feature evidence and a
bounded UDP miss. The default-resource test initially found zero UDP backup
services. Compiler Debug/Instant diagnostics and an incremental Rust compiler
ICE are separate implementation/build errors, not protocol REDs. The ICE was
avoided with per-process `CARGO_INCREMENTAL=0`; no shared cache was deleted.

The completed run stores, keys and invitations are never copied into repository
evidence. Owned Docker workers/namespaces and temporary phone storage were
removed. Only public fixture metadata, logs, results and captures remain.
Independent review is recorded in `review.md` at completion.

Repository log copies normalize trailing whitespace and terminal blank lines; original OS-temporary logs remain unchanged.
