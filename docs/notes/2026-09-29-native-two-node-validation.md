# MIOPUNCH-15: native two-node integration result

This is an implementation result for [MIOPUNCH-15](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/a451a208-eceb-40b4-817c-3893a9219a87), not a new product decision.

## Reproduce

Use Rust 1.96 or newer. The verified toolchain was 1.98.1. From the repository root, run `cargo test --locked --test native_probe`. The test launches separate native server and client processes with independent data directories. It performs a real UDP QUIC/TLS ping through iroh, then through `p2panda-net::Endpoint`, checks a reachable address paired with the wrong target identity, and restarts a node from its saved Ed25519 key. Local socket permission is required.

For a manual run, start `cargo run --locked -- serve wrapped /tmp/miopunch-15-server`. The process prints `READY <node-id> <udp-address>`. In another terminal run `cargo run --locked -- send wrapped /tmp/miopunch-15-client <node-id> <udp-address>`; it prints `PONG`. Replace `wrapped` with `raw` to run the direct iroh path. The server handles one exchange and exits. Reusing the server data directory retains its node identity. Use separate directories for separate nodes. The paths here are examples; the commands create the directories and a private `node.key` file.

## Locked combination and selected seam

`Cargo.lock` resolves one `iroh 1.2.0` and one `iroh-base 1.2.0` shared by the probe and `p2panda-net 0.7.1`. It resolves `iroh-relay 1.2.0`, `iroh-dns 1.3.0`, `noq-proto 1.3.0`, and `p2panda-core`/`p2panda-store 0.7.1`. The local p2panda-net source is pinned to upstream commit `083b48215964e92a564f3c83a1c607b00e94aa64`, with its small delta described in [the vendor note](../../vendor/p2panda-net/UPSTREAM.md). Build again with `cargo check --locked` or `cargo test --locked`.

The selected seam is a reusable closure on the p2panda endpoint builder. It configures iroh **before bind**, after p2panda has installed its key, relay mode, and address-book lookup. A prebound endpoint would shift accept-loop and close ownership away from p2panda and cannot be rebuilt from a one-shot builder after actor restart. Here p2panda continues to own the one accept loop and shutdown; the closure is held in cloneable actor arguments so a supervisor can call it on a fresh builder. The probe starts only AddressBook and Endpoint, not the high-level Node, mDNS, discovery, gossip, or sync.

When the address book has no iroh transport entry, `connect(node_id, ...)` now passes an identity-only `EndpointAddr` to iroh. In the wrapped probe, iroh's separate `MemoryLookup` supplies the explicitly given UDP address, and the same Ed25519 bytes become p2panda's and iroh's node IDs. A reachable address associated with another target ID fails the authenticated handshake. The probe configures the existing `noq-proto::congestion::Bbr3Config` through iroh's QUIC transport builder, and the connection succeeds with that configuration. It does not measure congestion behavior.

This proves a narrow explicit-address lookup route, not unknown-member discovery, NAT traversal, admission, or normal network synchronization. The key file is durable across an ordinary restart; crash atomicity and broader identity storage policy belong to later persistence work. The probe currently binds loopback UDP for a controlled first check.
