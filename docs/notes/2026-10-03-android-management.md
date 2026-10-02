# Android management and lifecycle through the shared node

[MIOPUNCH-53](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/393ac2c9-6ac0-47b8-a3b1-586f2757c0af)
uses the existing native JSON-lines [management shell](2026-10-03-desktop-management.md)
on an actual Pixel 6a. Creation, manual joining, precode issuance/redemption,
daily management, genesis elevation, saved resources and interrupted admission
all call the same Rust `NativeNetwork` implementation. The Android shell has no
separate authorization, cryptography or storage model.

## Reproduce

The Android Rust target must be installed. Build from the repository root with
an Android NDK, then run against a USB-connected Android device and a Linux
peer on a mutually reachable LAN:

```sh
rtk proxy python3 -B scripts/build_android_node.py --ndk /path/to/android-ndk --output /tmp/android-build-new
rtk cargo build --locked --bin miopunch-native-probe
rtk proxy python3 -B scripts/check_android_management.py --output /tmp/android-check-new --android-ip PHONE_LAN_IP --linux-ip LINUX_LAN_IP
```

The builder uses the Linux NDK compiler when available. In the observed WSL
setup it reuses the installed Windows NDK's target sysroot/libraries with native
Clang and Rust lld, without executing Windows compiler binaries. It records
actual tool versions and the build exit. The observed combination is Rust
1.98.1, Clang 14.0.6, NDK 29.0.14206865, `aarch64-linux-android`, API 24 target,
executed on Pixel 6a Android 16/API 36. This qualifies that actual target/runtime
combination, not every Android API level or architecture.

The runtime runner accepts `--adb`, `--serial`, and prebuilt binary paths. Its
ordered cases are `first`, `restart`, `maintenance_entry`, `pause`,
`network_change`, and `precode`; `--case` selects one. The first check creates
on Android and joins from Linux, then reverses the direction, using real
management and genesis gates. The priority check kills the actual Android
process while materials are pending, resumes the same approval, and kills it
again after completion. Later cases add one lifecycle fault at a time.

The runner creates a unique mode-0700 directory under `/data/local/tmp` and a
private host output directory. It transfers trusted invitation files, while
only public status/results enter `processes.log`. It restores enabled Wi-Fi
in a `finally` block during the network case and removes its own remote binary,
node material and processes. Do not publish the host output's private fixture
files; only the selected evidence logs/results are intended for sharing.

## Lifecycle entry and ownership

```text
miopunch-native-probe network maintain DATA_DIR [BIND_IPV4]
```

This additional JSON-lines entry owns the already joined/created node through
`NativeNetwork::maintain_network`. It runs the existing single coordinator,
real LogSync, contact persistence, resource qualification and the saved
`ResourceConfig.maintenance` budget. It introduces no Android identity role or
second contact/retry loop. Management and maintenance modes own a data directory
sequentially; stop one before starting the other.

| Operation | Behavior |
| --- | --- |
| `status` | Local maintenance state, public identity, saved-record count, last source-scoped sync, current neighbors, actual attempts and next delay. |
| `contact` with `address` | Offer an untrusted reachability hint to the shared coordinator; it must authenticate and authorize the peer. |
| `network_changed` | Forward the platform event to both iroh's transport refresh and the shared node coordinator. |
| `suspend` | Close the endpoint/coordinator, retain durable membership, and return `Suspended` with no live neighbors. |
| `resume` | Reopen saved identity/material/resources/contacts and start a fresh runtime. |
| `stop` or EOF | Disconnect and close storage; do not leave the network. |

The ready/resume result includes the current address and a random runtime
`generation`. A new runtime cannot be mistaken for a resumed old connection.
Neighbor `connection` values are the underlying connection's lifetime-local
stable IDs, not persistent session identifiers or application-resume tokens.
Historical `last_sync` remains source-scoped history even when current state is
`TemporarilyUnreachable`. A stopped OS process cannot deliver fresh status while
it is frozen; the remote peer observes timeout, and resumed event processing
reports/reestablishes current participation.

Android's selected netwatch backend does not supply netlink route callbacks.
The explicit lifecycle input is the platform seam for a future app's network
callbacks. ADB drives that same input in this test shell. The test verifies
saved contacts recover without another hint after suspend or process restart;
it does not make users choose a signaling topic, punching sequence or transport.

## Android publication correction

The first reverse-direction device check failed while saving admission material:
Android denied `hard_link` in the writable shell directory. An isolated device
check confirmed hard-link denial while rename succeeded. `native_files` now
uses Android `renameat2(RENAME_NOREPLACE)` for publication of a fully flushed
private temporary file. It preserves atomic publication and refuses an existing
target, then the existing caller flushes the directory. This is the same
no-replace primitive already used for native network-directory publication.
Replacement remains rename; SQLite retains its existing WAL/FULL semantics.
Other Unix and Windows publication branches are unchanged. Actual process-kill
recovery is qualified; physical power loss is not.

## Actual capability boundary

This is a foreground ADB shell executable, with shell-owned storage, not an
installed application, app-private sandbox, service, UI or wake-lock policy.
Explicit suspend/resume, real `SIGSTOP`/`SIGCONT`, real `SIGKILL`, and actual
Wi-Fi disable/enable are exercised. The old connection becomes unavailable and
later participation uses a new connection without repeating membership approval.
It does not promise continuous operation under Doze, background app quotas,
force-stop, OEM power management, or survival of every network transition.

The shell has no Java/NDK application context. An initial default-resource run
exposed the Android HTTPS platform verifier's missing-context panic, independently
of the file-publication bug. The controlled runner therefore explicitly imports
an empty replacement auxiliary-resource table before endpoint startup and uses
real LAN UDP. This avoids claiming a public HTTPS relay is qualified by a shell
without its required app context; it is not a new default or a silent fallback.
An installed app must provide the platform verifier context before enabling
those HTTPS resources. Public-resource runtime on Android, cellular data/NAT
success, app installation, reboot, app-private keystore integration and service
ACL/TUN/UI remain unqualified. Wi-Fi interruption/recovery is actual device
network-fault evidence, not cross-carrier traversal evidence.

See [the evidence index](../evidence/mio53-android/README.md) for observed results,
the original failures, affected regressions and independent reviews.
