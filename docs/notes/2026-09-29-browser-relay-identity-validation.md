# MIOPUNCH-16: browser relay and persistent identity validation

This is the narrow validation result for [MIOPUNCH-16](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/aee94cf2-4402-4c5d-8be0-a4edda96f61f). It does not establish browser admission, authorization, record synchronization, UDP hole punching, WebRTC, or background operation.

## Reproduce

The verified tools were Rust 1.98.1, `wasm32-unknown-unknown`, `wasm-bindgen-cli` 0.2.129, Playwright Core 1.63.0, and Chromium 148.0.7778.96. `Cargo.lock` and `browser-probe/harness/package-lock.json` pin Rust and JavaScript dependencies. From the repository root:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo build --locked -p miopunch-browser-probe --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir browser-probe/pkg target/wasm32-unknown-unknown/debug/miopunch_browser_probe.wasm
cargo build --locked --bin browser-probe-native
npm ci --prefix browser-probe/harness
node browser-probe/harness/node_modules/playwright-core/cli.js install chromium
node browser-probe/harness/run.mjs
```

The Playwright CLI installs the Chromium build pinned by Playwright Core 1.63.0. The harness uses that build, an existing cached Chromium build, or `PLAYWRIGHT_CHROMIUM_EXECUTABLE`. It starts a local HTTP relay on a free port, a native endpoint with IP transports removed, and a local static page. Chromium opens the page, connects over the self-hosted relay, sends `PING` through an authenticated iroh QUIC/TLS session and receives native `PONG`. It then reloads and repeats the exchange with the same browser node ID. After clearing the page origin's local storage and reloading, it observes a different browser ID and repeats the exchange. It checks that the browser made no requests outside the page and selected relay. The test process stops the relay and native node afterward.

The observed run printed `FIRST_EXCHANGE`, `RELOAD_IDENTITY`, `CLEARED_IDENTITY`, and `PASS`; the native process printed three `PONG` results. The browser and native IDs differed, the two pre-clear browser IDs matched, and the post-clear ID differed. Because the native endpoint had no IP transport, these exchanges depended on the relay. `Endpoint::online()` only established relay registration; the echo checked actual peer communication. Chromium used `ws://127.0.0.1:<ephemeral-port>/relay`; no default relay was configured or observed.

`cargo check --locked --workspace`, wasm `cargo check` and build, `cargo fmt --all -- --check`, and the browser harness passed. The first parallel `cargo test --locked --workspace` run timed out in the vendored `iroh_endpoint::tests::establish_connection`; that test passed alone immediately afterward. The full suite then passed with `cargo test --locked --workspace -- --test-threads=1`: 4 native probe tests, 40 vendored unit tests with 1 upstream ignored test, 2 vendored integration tests, and 7 doc tests. The intermittent parallel timeout remains a test-runner limitation; this ticket did not change that upstream test.

The thin page stores the browser's 32-byte Ed25519 secret in that origin's `localStorage`; it loads or generates the key in the browser and does not submit it to the page server. It compares p2panda-core's verification key with iroh's public key from the same bytes before binding. This proves the identity conversion and reload route, not production-grade browser secret storage or crash-safe persistence. The server serves only static HTML, JavaScript, and wasm; the self-hosted relay sees encrypted packets, not the node secret or plaintext message.

## Dependency and adapter result

The browser crate builds and runs with `iroh 1.2.0` (`default-features = false`, `tls-ring`), `iroh-base 1.2.0`, `p2panda-core 0.7.1`, and `p2panda-store 0.7.1` with store defaults disabled. `cargo tree -p miopunch-browser-probe --target wasm32-unknown-unknown -e normal,build` has no `sqlx`, SQLite, `ractor`, or `mio` branch. The reusable store crate exposes `OperationStore`, `LogStore`, `TopicStore`, `AddressBookStore`, and `Transaction` interfaces, but this ticket only compiles them; it does not yet save or synchronize control records.

The existing `p2panda-net::iroh_endpoint` feature selects `address_book`, and that selects `p2panda-store/sqlite` and `macros` even with net defaults disabled. Its address-book builder fixes `SqliteStore`; its endpoint bind path calls native-only `bind_addr` twice. A direct wasm check of `p2panda-net --no-default-features --features iroh_endpoint` failed first in `mio` because the selected Tokio net feature does not support `wasm32-unknown-unknown`. The feature tree also contains SQLx SQLite. Thus the complete native endpoint actor is not directly portable by only changing the top-level feature flag. The high-level `p2panda::Node` and `p2panda-sync` source also select default SQLite store features; they were not included in the browser build.

For subsequent browser synchronization, the implementation needs a browser storage adapter implementing the required log, topic, operation, cursor, address-book, and transaction contracts with durable commits; browser-safe runtime/actor configuration; and a protocol gate that checks the same verified membership and authorization records as native clients. These are integration interfaces to validate in later tickets, not new product behavior or a choice of IndexedDB versus OPFS. The browser's current echo grants no network membership.
