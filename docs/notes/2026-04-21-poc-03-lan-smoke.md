# POC-03 LAN 3-process smoke (mesh-first + MQTT fallback)

This note documents how to run the POC-03 control-plane smoke on a real LAN with **three processes**:

- **A**: sender (runs `--smoke`)
- **B**: forwarder (mesh-only forwarding A↔B↔C)
- **C**: receiver (handles `smoke_echo_req`, replies `smoke_echo_resp`)

The smoke validates:

- bounded flooding forwarding with `H=3` (A→B→C)
- signature transcript covers `dst_peer_id` (receiver verifies signature)
- mesh-first request delivery
- MQTT fallback (after 1s) does not cause duplicate side effects (receiver dedup by `msg_id`)

## Prereqs

- A reachable MQTT broker (e.g. `tcp://192.168.1.10:1883`)
- UDP connectivity between the three hosts on the chosen ports
- A shared `net_secret_hex` across all nodes

## 0) Pick secrets (example)

Use any 32-byte hex strings you want (examples only):

```bash
export NET_SECRET_HEX=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef

export SEED_A_HEX=000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
export SEED_B_HEX=1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100
export SEED_C_HEX=ffffffffffffffffffffffffffffffff00000000000000000000000000000000
```

## 1) Print identities (peer_id + pubkey)

Run once on each node to print `self_peer_id` and `self_pub_b64url`:

```bash
go run ./tools/miopunch-cp-smoke \
  --net-secret-hex "$NET_SECRET_HEX" \
  --seed-hex "$SEED_A_HEX" \
  --print-identity
```

Repeat with `SEED_B_HEX` and `SEED_C_HEX`. Copy these values:

- `PEER_A`, `PUB_A`
- `PEER_B`, `PUB_B`
- `PEER_C`, `PUB_C`

## 2) Start B (forwarder)

Topology: `A ↔ B ↔ C` (no direct A↔C neighbor).

On node B:

```bash
go run ./tools/miopunch-cp-smoke \
  --net-secret-hex "$NET_SECRET_HEX" \
  --seed-hex "$SEED_B_HEX" \
  --listen-udp 0.0.0.0:9002 \
  --neighbor "$PEER_A=<A_IP>:9001" \
  --neighbor "$PEER_C=<C_IP>:9003" \
  --peer "$PEER_A:$PUB_A" \
  --peer "$PEER_C:$PUB_C" \
  --mqtt-url tcp://<MQTT_IP>:1883
```

Leave it running.

## 3) Start C (receiver)

On node C:

```bash
go run ./tools/miopunch-cp-smoke \
  --net-secret-hex "$NET_SECRET_HEX" \
  --seed-hex "$SEED_C_HEX" \
  --listen-udp 0.0.0.0:9003 \
  --neighbor "$PEER_B=<B_IP>:9002" \
  --peer "$PEER_A:$PUB_A" \
  --mqtt-url tcp://<MQTT_IP>:1883
```

To **force MQTT fallback** (so you can observe dedup across mesh+MQTT), add a response delay:

```bash
  --response-delay 2s
```

You should see `echo_req recv` printed **once** at C (even if fallback triggers).

## 4) Run A (sender smoke)

On node A:

```bash
go run ./tools/miopunch-cp-smoke \
  --net-secret-hex "$NET_SECRET_HEX" \
  --seed-hex "$SEED_A_HEX" \
  --listen-udp 0.0.0.0:9001 \
  --neighbor "$PEER_B=<B_IP>:9002" \
  --peer "$PEER_C:$PUB_C" \
  --mqtt-url tcp://<MQTT_IP>:1883 \
  --smoke \
  --smoke-dst-peer-id "$PEER_C"
```

Expected outputs:

- If C has no delay: A prints `response received (mesh-first)`.
- If C uses `--response-delay 2s`: A prints `MQTT fallback` then later `response received`.
- If the request arrives at C via MQTT (e.g. no working mesh path), C replies via MQTT and A prints `echo_resp recv: via=mqtt`.

Each process prints a final `facts:` line on exit, including:

- `mesh_forward_drops` (required by POC-03 diagnostics)

## Notes

- This harness lives under `tools/miopunch-cp-smoke/` on purpose (it’s a repo-local smoke tool, not a product `miopunch` CLI entrypoint, and does not require modifying `miopunch-lab`).
- For convenience (local smoke only), the tool also accepts `--peer-seed-hex` to derive peer IDs + pubkeys from seeds, so you don’t need to copy `PUB_*` values around.
