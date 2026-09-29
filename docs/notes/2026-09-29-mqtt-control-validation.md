# MIOPUNCH-20: MQTT control carrier validation

The [MIOPUNCH-20 ticket](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/f3c68d7a-5794-4897-b953-c8c8a9353c9b) is the authority for scope. This note records the concrete test carrier and its observations. It is not a general membership or business transport.

## Carrier and trust boundary

`src/mqtt_transport.rs` implements iroh's `CustomTransport` through `rumqttc` 0.25.1 and a real MQTT 3.1.1 broker. Each endpoint creates a fresh random 128-bit route when it binds and subscribes to `miopunch/test/<route>/packet`. One QoS 0, non-retained publish contains one complete QUIC datagram. The payload is `MIO1` (4 bytes), the sender's 16-byte route, then the datagram. There is no application-level fragmentation or reassembly. Empty/overlong datagrams, malformed envelopes, retained publishes and full receive queues are bounded and rejected; QUIC handles loss of a whole publish. The datagram limit is 2048 bytes, the MQTT packet limit is 4096 bytes, and each adapter queue has 64 entries.

The broker and any wildcard subscriber can see the destination topic, source route, MQTT client ID, packet sizes, timing and public QUIC Initial material, including the test ALPN. The adapter places no node ID, invitation or network admission secret in its own clear envelope. The route is a delivery address, never a credential. The probe disables iroh IP and relay transports, checks the selected custom path, and opens only `miopunch/control-probe/1`. The client pins the server's iroh public key in its connect target; the server checks the expected client's `remote_id()` before accepting the control stream. The only application message is `PING`/`PONG` after the authenticated QUIC handshake. MQTT delivery and TLS peer identity do not grant network membership.

`miopunch-native-probe identity DATA_DIR` creates or reads the existing mode-0600 node key and prints its public identity. The runnable MQTT forms are `serve mqtt SERVER_DIR BROKER_ADDR EXPECTED_CLIENT_ID`, `send mqtt CLIENT_DIR SERVER_ID SERVER_ROUTE BROKER_ADDR`, and `cancel mqtt DATA_DIR BROKER_ADDR`. `serve` prints `READY SERVER_ID SERVER_ROUTE` after SUBACK. The route changes on each bind. The adapter is exported from `src/lib.rs` for later composition, while the CLI remains a restricted test harness.

## Reproduction

Use a dedicated loopback broker; `eclipse-mosquitto:2` was already available locally. From the repository root:

```sh
rtk proxy docker run --rm -d --name miopunch-mio20-mqtt -p 127.0.0.1::1883 -v "$PWD/scripts/mosquitto-test.conf:/mosquitto/config/mosquitto.conf:ro" eclipse-mosquitto:2
rtk proxy docker port miopunch-mio20-mqtt 1883/tcp
rtk cargo build --locked --bin miopunch-native-probe
rtk proxy env MIOPUNCH_MQTT_BROKER=127.0.0.1:PORT cargo test --locked --test mqtt_probe -- --test-threads=1 --nocapture
rtk proxy python3 scripts/mqtt_loss_probe.py 127.0.0.1:PORT
rtk proxy python3 scripts/mqtt_loss_probe.py 127.0.0.1:PORT duplicate
rtk proxy python3 scripts/mqtt_disconnect_probe.py
rtk proxy docker rm -f miopunch-mio20-mqtt
```

Replace `PORT` with the port printed by Docker. The broker test file returns early when `MIOPUNCH_MQTT_BROKER` is absent, so run the targeted command above for the real acceptance check. The fault proxy forwards through the real broker while dropping or duplicating exactly the first client MQTT PUBLISH; both modes use a valid QUIC datagram. The disconnect probe creates and removes its own isolated broker container. MQTT rides TCP and the adapter never fragments a QUIC datagram across publishes: losing a TCP segment either causes retransmission within TCP or a broken broker connection, while losing a whole publish is the carrier-level loss tested here.

## Observed result, 2026-09-29

- Real broker on `127.0.0.1:43255`: independent persisted node keys completed an iroh handshake and `PING`/`PONG` over the custom path in 127 ms on one run. Client counters: 15 publishes / 6685 QUIC bytes sent, 12 / 5667 received. Server counters: 12 / 5667 sent, 13 / 6605 received. Counts can differ at close because QUIC retransmits and the endpoints close promptly after one exchange.
- Wrong server key at the reachable route failed. Another MQTT subscriber with its own valid iroh key could not pass the server's expected-peer check or obtain `PONG`. A wildcard subscriber injected a truncated envelope, duplicate short datagrams, and observed 99 publishes; the largest observed MQTT payload was 1409 bytes. It saw no `PING` or `PONG` plaintext. The authorized client still completed control afterward.
- Explicit endpoint cancellation returned `CANCELLED`. Rebinding one adapter after close acquired a distinct route. Stopping an isolated broker after `READY` produced a nonzero server exit and `MQTT carrier closed before control exchange` in 259 ms. Dropping the first client PUBLISH (1278 MQTT bytes) still resulted in `PONG` through QUIC retransmission; duplicating that valid 1278-byte publish also produced one bounded `PONG` exchange.

This proves the local real-broker path and these controlled failure cases. It does not validate unknown-peer discovery, admission proof, normal business traffic, carrier switching, hostile broker traffic analysis, or delivery across other networks. Those remain with their own dependent tickets.
