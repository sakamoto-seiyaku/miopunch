# MIOPUNCH-28 native network create and restart

The public entry is `miopunch-native-probe network create DATA_DIR`, `network status DATA_DIR`, and `network run DATA_DIR [BIND_IPV4]`. `run` binds an actual iroh endpoint from the saved device key and exits cleanly on Ctrl-C. Creation uses a new data directory. `status` replays and verifies the persisted signed management history, then prints only the network root public key, canonical Create hash, node public key, member roles, and applied record count.

Reproduce the create, process stop, and restart check on Linux with loopback sockets permitted:

```bash
rtk cargo build --locked --bin miopunch-native-probe
rtk proxy bash scripts/native_network_smoke.sh target/debug/miopunch-native-probe
rtk cargo test --locked --test native_network -- --test-threads=1
```

The smoke runner uses a temporary directory, compares public output across two new processes and an endpoint run, then removes the temporary data. The targeted test also checks that a new root creates a different network, saved root plus pinned Create stays stable, secrets do not appear in status, a wrong root or alternate signed Create is refused, and an unauthorized management record does not alter the view. The endpoint's live iroh ID must equal the saved node key's public identity.

On 2026-09-29, the smoke runner printed `RECORDS 1`, the same public node ID in `NODE`, `MEMBER`, and `READY`, then `RESTART_OK`. The targeted suite passed 5/5. With a dedicated local Mosquitto broker, `cargo test --locked --workspace -- --test-threads=1` passed 164 tests, with three pre-existing upstream tests ignored. `cargo check --locked --workspace --tests`, `cargo fmt --all -- --check`, and `bash -n scripts/native_network_smoke.sh` also passed. The broker was stopped after the run.

The data directory stores `node.key` (device Ed25519 private key), `genesis.credential` (root Ed25519 private key plus canonical Create hash), `network.anchor` (root public key plus canonical Create hash), `admission.secret` (shared secret), and `management.sqlite` (signed records and applied authorization view). The three secret files are created mode `0600` inside a mode `0700` directory on Unix. The root credential and shared admission secret are independent. An actual backup medium, key unlock policy, member admission, and automatic peer contact belong to later work; this entry only creates the first node and starts its endpoint.

Review follow-up: directory publication now uses atomic no-replace rename on Linux, Android, and Apple targets, so a concurrent creator cannot overwrite even an empty target directory. Other Unix targets fail safely until an equivalent operation is available. A targeted unit check for that race, all five native network integration cases, and the public smoke runner passed again after the change. The stored anchor and genesis credential use separate typed encoders and decoders to keep their field layouts together.
