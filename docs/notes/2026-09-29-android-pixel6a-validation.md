# MIOPUNCH-17: Pixel6a Android node validation

This records the narrow platform check for [MIOPUNCH-17](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/465b71f9-0400-458a-b50e-7de7deef61a6). It uses the same `miopunch-native-probe` p2panda endpoint and iroh QUIC/TLS protocol as the [Linux two-node check](2026-09-29-native-two-node-validation.md). The only new probe input is an optional IPv4 bind address. Its default remains loopback; `0.0.0.0` enables a LAN peer to reach a test node. A server handles one `PING`/`PONG` exchange and exits.

## Reproduce on a connected Android device

Build the native Linux binary and the Android `aarch64-linux-android` binary from the locked workspace. The Android build needs that Rust target and an NDK linker for the target; for example, with a Linux-host Android NDK:

```sh
rustup target add aarch64-linux-android
export ANDROID_NDK_HOME=/path/to/android-ndk
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
cargo build --locked --bin miopunch-native-probe
cargo build --locked --target aarch64-linux-android --bin miopunch-native-probe
adb push target/aarch64-linux-android/debug/miopunch-native-probe /data/local/tmp/miopunch-probe
adb shell chmod 700 /data/local/tmp/miopunch-probe
```

In one terminal, run `adb shell /data/local/tmp/miopunch-probe serve wrapped /data/local/tmp/miopunch-node 0.0.0.0`. The `READY <node-id> <listed-ip>:<udp-port>` line provides the authenticated target ID and listening port. With a wildcard bind, iroh may list a cellular or VPN interface first; do not assume `<listed-ip>` is reachable from Linux. Obtain the phone's actual Wi-Fi IPv4 address with `adb shell ip -4 addr show wlan0` and pair it with the `READY` port. In another terminal, run `target/debug/miopunch-native-probe send wrapped /tmp/miopunch-linux <node-id> <phone-wifi-ip>:<udp-port> 0.0.0.0`; success prints `PONG`. Both devices must be on a LAN where the Linux process can reach the phone's UDP port. The data directories must be distinct. No default relay or discovery service is used.

Restart the phone server with the **same** data directory and compare its `READY` node ID with the first run. It must match; the UDP port may change. Send again using the new port. For the wrong-target check, obtain a second, real node ID from another probe data directory, then pass that ID with the phone's reachable UDP address to the Linux `send` command. It must exit nonzero with a TLS/QUIC peer-certificate error and without `PONG`; the correct ID should still reach the phone afterward. For the storage failure check, create a separate phone directory, remove its write permission, and start `serve` with that directory. It must exit nonzero with `PermissionDenied` before printing `READY`. Restore permissions and remove only those temporary test files when finished.

## Observed run

On 2026-09-29, an ADB-connected Pixel6a running Android 16 and a Linux reference process communicated over the phone's Wi-Fi address `192.168.4.151`. The phone printed node ID `f7d585304d7c95e839395b1a18357f47ff88919ebc382ba3a050e6ea4ffb217b` on three process starts, with UDP ports `49169`, `51252`, and `46906`; the Linux client printed `PONG` on all three exchanges, including after the reviewed CLI parsing change. The second run first received a connection addressed to a different real node ID at the same reachable phone address; that attempt exited 1 with `invalid peer certificate: UnknownIssuer` and no `PONG`. An isolated phone data directory with mode `0500` then caused key creation to exit 1 with `PermissionDenied` and no `READY`. The saved `node.key` was 32 bytes with mode `0600`.

The locked versions were Rust 1.98.1, iroh 1.2.0, p2panda-core/net 0.7.1 and noq-proto 1.3.0. This run used the Android NDK 29.0.14206865 sysroot already installed on the Windows side of WSL, native Linux Clang 14, and Rust's `rust-lld` because this host lacked a Linux-host NDK installation. The resulting Android binary was built for API 24 and executed on the Pixel6a. The ordinary Linux and Android target builds and the targeted wildcard-bind integration test passed.

`cargo check --locked --workspace`, `cargo fmt --all -- --check`, and all six native probe tests passed. The sixth test ensures an invalid CLI mode never creates a node key. The first three full serial workspace runs had intermittent failures in different vendored p2panda tests. Temporary error-chain logging traced the common cause to listener bind `Address already in use (os error 98)`: the vendored test fixture chose random ports from `49152..65535`, overlapping this host's `32768..60999` ephemeral-client range. Moving only that fixture's listener range to `20000..30000` preserved its seeded deterministic test and made the next complete serial workspace suite pass: six native probe tests, 40 vendored unit tests passed with one upstream ignored, two integration tests, and seven doc tests. The temporary diagnostic logging was removed. The fixed range avoids this host's collision source but is not a reservation against unrelated processes on every host.

The temporary Pixel6a binary and test data directories, including the test key, were removed after the checks. The Linux test key directories under `/tmp` were removed too.

This is a foreground ADB shell process with a shell-owned key under `/data/local/tmp`, not an installed Android app, service, or app-private identity store. It proves process restart from that file and LAN UDP reachability. App lifecycle, device reboot, sleep/background behavior, cellular paths, NAT traversal, membership, authorization, and control-state synchronization remain outside this check. The shell has only the existing probe echo protocol and does not implement separate network or authorization rules.
