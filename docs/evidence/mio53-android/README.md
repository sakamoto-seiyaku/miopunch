# MIOPUNCH-53 Android evidence

Delivery and reproduction: [Android management note](../../notes/2026-10-03-android-management.md).
The live ticket and parent specification remain authoritative. This directory
contains selected secret-free runtime evidence, not node keys or invitation files.

## Final results

- [Validation receipt](validation.json): actual Android/Linux build exits 0;
  final runtime runner exit 0 with all six cases passed.
- [Platform results](platform/results.json), [runtime log](platform/processes.log),
  [device environment](platform/environment.txt), and [cleanup](platform/cleanup.json).
  Actual Pixel 6a Android 16/API 36, independent Linux peer, real Ed25519,
  SQLite, TLS/QUIC and LogSync. The runner uses an explicit LAN-only resource
  replacement, not a simulated network or crypto success path.
- [Build metadata](build/build.json) and [build log](build/build.log): locked
  Android target using the available NDK sysroot, native Clang and Rust lld.
- [Affected native regression log](regressions/affected.log): 56 passing entries
  across admission completion, daily management, manual admission, native network
  and native probe. This includes three inert crash-worker entry points; their
  parent checks execute real subprocess interruption.
- [Native all-target check](regressions/check.log), [WASM check](regressions/wasm.log),
  [format check](regressions/fmt.log): exit 0. Unchanged slow NAT and unrelated
  platform/service matrices were skipped under the standing user policy.

The final matrix first runs creation/join plus management in both platform
directions, then the priority Android SIGKILL/pending-material recovery check.
It subsequently exercises the shared maintenance entry, explicit suspend,
actual OS STOP/CONT, process kill and automatic saved-contact restart, actual
Wi-Fi loss/recovery, and Android precode count/pending-delivery restart.
Wi-Fi is restored and the owned device directory/processes are removed.

## Earlier failures and corrections

- [First RED](first-red/results.json), [log](first-red/processes.log): Android
  creation/Linux admission worked, but Android admission-material publication
  failed with permission denied. The same initial run exposed the separate
  missing Android app context for default HTTPS platform verification.
- [Narrow diagnostic](diagnostic/processes.log): the admission error is
  `Node(Io(PermissionDenied))`. A disposable device `ln` also failed while
  `mv` succeeded. No error details or secret material were retained in production
  command output; the temporary diagnostic was removed.
- [Publication GREEN](publication-green/results.json): the original reverse
  platform scenario passes after Android atomic no-replace rename publication.
- [Wrong LAN hint](wrong-lan-hint/results.json): the controlled LAN runner
  initially received the gateway address instead of the Linux interface address.
  It correctly reported temporary unreachability; correcting the fixture hint
  passed. This is not represented as a runtime product success.
- [Lifecycle entry RED](entry-red/results.json) and [GREEN](entry-green/results.json):
  the public CLI rejected the missing entry before implementation and accepted
  the shared maintenance operation afterward.

Android app-private storage, installed-app/JNI HTTPS resources, Doze/service
uptime, cellular traversal, arbitrary NAT and physical power loss are not
qualified by these foreground shell checks. See the delivery note for details.

## Independent review

The required Standards and Spec review reports are added after review of the
committed change against `db528ca1e6d0f36941ddb00687d838e9b1b21504`.
