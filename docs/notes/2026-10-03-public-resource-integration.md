# Linux / Android public resource integration

MIOPUNCH-64 supplements the controlled facilities of MIOPUNCH-38/42 and the
physical-device LAN run of MIOPUNCH-53. The shared node now performs actual
STUN Binding observations, upgrades an authenticated MQTT control connection
to an allowed public relay on that same Connection, and supplies Android's
real JNI context through a foreground test shell. The normal node / Endpoint /
Connection / stream interfaces still own authorization and persistence.

The [receipts](../evidence/mio64-public-resources/README.md) distinguish observed
success, external failures and local prerequisites. They retain the original
Android context panic, unavailable public brokers, STUN failures and the
closed-path snapshot regression.

## Actual services and topology

Linux runs in WSL2; the physical Pixel 6a runs Android 16, API 36, aarch64.
Both have private IPv4 addresses on the same /24 Wi-Fi network. There is no
active independent cellular connection in this environment. Every accepted
public data session disables UDP direct transport and excludes both devices'
local TCP addresses. Its sole active path is the official number0 relay at
`https://aps1-1.relay.n0.iroh.link./`, with ordinary embedded WebPKI verification.
This proves an actual public relay data path; it does not prove cross-network
direct traversal or a carrier-wide NAT capability.

The default resource table is unchanged. Initial admission, exactly one
precode redemption and a business PONG use the actual default public relay.
The later explicit replacement selects EMQX `mqtt://broker.emqx.io:1883/`,
Cloudflare `stun+tcp://turn.cloudflare.com:3478/`, Cloudflare
`stun://stun.cloudflare.com:3478/`, and the same approved HTTPS relay.
EMQX's [official dashboard](https://www.emqx.com/en/mqtt-dashboard) documents
this public broker. An actual Mosquitto port-1883 CONNECT returned EOF and
HiveMQ timed out; EMQX returned CONNACK and carried the protected bootstrap.
An independent verified-TLS Mosquitto port-8886 probe also returned CONNACK,
but the current MQTT adapter has no TLS transport, so that probe is not claimed
as node integration. [Mosquitto's service description](https://test.mosquitto.org/)
identifies its test ports and availability limits. These are best-effort public
facilities, and one failure says nothing about all network members' presence.

Public MQTT is plaintext TCP at its outer carrier layer. Endpoint identity,
private topic derivation, SCRAM admission and inner QUIC/TLS remain the existing
protected workflow. MQTT packet counters settle before business traffic and
remain unchanged during actual business PING/PONG. Neither business traffic
nor LogSync is qualified by a signaling-only connection.

## Recorded integration decisions

1. STUN and QAD are separate protocols. Only HTTP(S) resources enter the QAD
   relay map. A `stun://` observation sends RFC 8489 Binding from the retained
   Endpoint IPv4 UDP socket. It validates a random transaction and the chosen
   response source; retransmits at most three times within a three-second
   bound; caps pending queries; and expires on a new observation, network
   change, socket close or 40-second age. A disabled observation sends nothing;
   a missing IPv4 UDP transport returns a local `Unsupported` error.
2. A `stun+tcp://` Binding uses the actual registered TCP transport's listening
   port, with the observation connection retained. Its result is a fresh,
   untrusted, TCP-specific candidate, never a UDP observation, relay address
   or proof of reachability. TCP peer, transaction and local port are checked;
   IPv4 and IPv6 XOR-MAPPED-ADDRESS responses are accepted. Closing the
   observation connection, changing the network, superseding the query,
   closing the Endpoint or 40-second age invalidates the result. Android's
   TCP hole punching remains disabled.
3. Cloudflare's public TURN listener responds to Binding as STUN, as its
   [official FAQ](https://developers.cloudflare.com/realtime/turn/faq/) explains.
   This run performs Binding only; no TURN allocation or TURN data relay is
   claimed. Linux and Android both receive validated TCP Binding responses.
   Both actual Endpoint UDP queries send three datagrams accepted by the kernel
   and time out in this environment. UDP failure is recorded independently.
   The wire checks follow [RFC 8489](https://www.rfc-editor.org/rfc/rfc8489).
4. Message bootstrap capability exchange is private version 2 and exchanges at
   most four allowed relay URLs in a bounded 1024-byte protected message. The
   dialing peer validates the relay path on the original restricted Connection;
   both peers agree before the ordinary admission gate closes signaling paths
   and permits application data. Mixed version-1/version-2 message bootstrap is
   unsupported and fails closed. Pure relay/native/browser connections keep
   their existing flow. This is no new public caller choreography.
5. Android needs a real JVM/context for its resolver. `MioPublicShell` starts
   under `app_process`, installs a retained JNI global context and runs the
   shared desktop command session from the native library. Certificates remain
   verified. This is a foreground physical-device test shell, with no installed
   application, product UI or background-service claim.
6. Fault injection uses an owned local HTTP CONNECT byte forwarder restricted
   to the approved official relay host/port. The actual peer is a public relay
   IP on 443; relay TLS passes unchanged and is verified by iroh. The forwarder
   implements no relay, terminates no TLS and fabricates no response. It lets
   the runner blackhole precisely this run's Linux relay bytes without touching
   MQTT, STUN, device network settings or other tests.
7. `Connection::paths` promises an open-path snapshot. Local Noq closure occurs
   before its delayed Abandoned event; that delay exposed a closed MQTT path
   immediately after data release. New snapshot construction filters actual
   closed Noq paths, including the first `paths_stream` snapshot. Previously
   captured snapshots retain their statistics. A deterministic public API test
   reproduces the original failure without awaiting the actor. Noq's
   `path_status` now rejects paths already marked abandoned, which makes its
   existing open-path handle check follow its documented contract.

## First check and failure check

Independent persisted identities complete default public-relay admission.
Actual public MQTT then discovers a peer whose supplied address contains only
its private signaling route. One unchanged Connection upgrades to the allowed
HTTPS public relay and exchanges business PING/PONG. A newly signed management
record is actually synchronized and saved on Android (record count 2 → 3).
Path snapshots and settled MQTT packet counts distinguish control from data.

The single injected fault blackholes only the Linux relay tunnel. The retained
business stream fails within its bound and the old connection terminates;
there is no signaling fallback. A fresh attempt demonstrably reaches MQTT
control, fails within its bound, and returns no business handle while the relay
remains unavailable. Restoring the tunnel and restarting Android retains its
node key, complete authorization, saved three records and exact resource
selection. New business traffic succeeds without approval or a second
redemption (precode uses remain 1). Disabling Observation, Signaling and
DataRelay survives restart, advertises no MQTT route, makes no STUN query and
prevents public data contact; restoring the saved profile succeeds.

The runner removes its owned remote directory, checks recorded remote PIDs,
closes its proxy sockets and leaves device network settings unchanged. The
temporary owned regression broker and preflight shell files are removed after
validation. Public-key identifiers in receipts are fixture identities, not
credentials. Private stores, invitation material, SSID/MAC details and API keys
are excluded from repository evidence.

## Reproduction

Build the normal Linux binary, then build the Android library and JNI shell:

```sh
rtk cargo build --locked --bin miopunch-native-probe
rtk proxy python3 scripts/build_android_node.py \
  --ndk /path/to/android/ndk --output /tmp/mio64-android-build \
  --jni-shell --d8-jar /path/to/android/build-tools/lib/d8.jar
rtk proxy python3 scripts/check_public_resources.py \
  --output /tmp/mio64-public-run \
  --android-shell /tmp/mio64-android-build/android-shell.jar \
  --android-ip DEVICE_IPV4 --linux-ip HOST_IPV4
```

Use an actual USB-connected Android device and reachable declared public
services. The runner records current DNS, protocol-specific results and public
relay tunnel peers. A service change or DNS/TLS error remains a failed run;
there is no injected success. Independent networks may be supplied, but this
receipt only qualifies the concrete topology described above. The affected
regression suite uses a separately owned controlled MQTT broker; that broker
never participates in the physical public-resource acceptance.
