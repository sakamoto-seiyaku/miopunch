# Pixel 6a and Y700 environment and SDK entry preparation

2026-10-05. [MIOPUNCH-82](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7178d29a-2fc8-45a4-b972-8ab23dd39588).
The original receipts are environment, build and entry-lifecycle results. The
later [read-only complete SDK story assessment](SDK-PUBLIC-RTC-ASSESSMENT.md)
separately records actual public TLS, member gate and RTC business evidence from
MIOPUNCH-73. Physical routing without overlay remains unqualified. Detailed site
parameters remain local evidence. Plane has only the user's approved scope, not
the site's private network details.

The user first accepted testing with the current WARP conditions, then selected
Pixel 6a ↔ Y700 as the preferred physical pair. The workstation supplies builds,
ADB control, code delivery and evidence; it is no longer a business endpoint.
Earlier workstation/WARP evidence remains in
[the original preflight](../wan-environment-2026-10-05/README.md) under its scope.
The user subsequently prohibited further standalone STUN checks/scans. All raw
observations below preceded that instruction and are retained, not repeated.

## Physical endpoints and bounded observations

| Device | Control | Physical IPv4 | Observed egress |
| --- | --- | --- | --- |
| Pixel 6a | existing USB ADB | `192.168.1.104/24`, WLAN via `192.168.1.1` | Bound Cloudflare IPv4 STUN: `111.197.237.227`; normal system TLS selects temporary global IPv6, `warp=off` |
| Y700 / TB322FC | existing network ADB; Tailscale only for control | `192.168.0.107/24`, WLAN via `192.168.0.1` | Bound IPv4 STUN and normal system TLS: `183.197.94.96`, `warp=off` |

Pixel initially had only its previous Wi-Fi saved. The target's actual scan name
was `CU_7WFb_5G` with WPA2-PSK security. After explicit user authorization and
credential delivery it connected to that target, retained USB control, and the
target is now saved. The credential was supplied through the local ADB shell's
stdin, omitted from host command arguments/evidence, and its temporary file was
deleted in `finally`; the intended phone saved-network entry remains. No Y700,
workstation or gateway VPN/network setting was changed.

Pixel's single Google IPv4 observation timed out; the retained narrow diagnosis
is `SocketTimeoutException`, caller 3010.833824ms, socket cleanup 0.349040ms.
One authorized comparison against the repository's already-enabled Cloudflare
observer succeeded with transaction and response-source validation: caller
304.236817ms, cleanup 0.377767ms, source `192.168.1.104:58509`, mapped
`111.197.237.227:36581`. This establishes an IPv4 observation, not peer NAT success.
Y700's independent bound observation also succeeded. The Google failure does
not establish that all Pixel IPv4 UDP is unavailable.

Pixel lacks `curl` (first exit 127 retained). A temporary Java HTTPS class used
default system certificate and hostname verification, without custom trust or
bypass flags; normal HTTP 200/TLSv1.3 and `warp=off` were observed on both devices.
All temporary device dex files were removed and independently checked absent.
Java TLS and raw UDP classes are probes, not product SDK actors or signaling.

The maintained STUN inventory later identified by the user/root is archived data
at `archive/legacy-go-openspec-2026-09-28/connectivity/stun_internal.go` (61 entries;
Google entries had been removed). Its use as explicit approved SDK configuration
does not import the old workflow or add all entries as product defaults. Future
SDK setup uses the agreed finite allowed group, at most the profile's existing
observer bound, with explicit matching numeric resource permissions. This helper
does not run that inventory as an availability loop.

## Formal browser entry

[android_pair_runtime.py](../../../scripts/android_pair_runtime.py) owns one test
page and local origin per actor, with ADB reverse delivering code/control only.
[android-pair-page.js](../../../browser-probe/harness/android-pair-page.js) imports
the formal WASM module; typed commands call `BrowserNetwork` and its member
objects. It implements no alternate identity, cryptography or product signaling.
Raw fixture configuration, invitation material and event replies stay in the
controller's private memory, not HTTP logs or public receipts.

Both actual device pages loaded the existing WASM artifact, exercised required
PC/DataChannel creation with negotiated ID 0/unordered/maxRetransmits 0, closed
the probe PC, and returned a real rejection for an unsupported controller command.
This package predated the new member-protocol final build; module loading and the
environment API probe do not qualify MIOPUNCH-73 or the SDK's effective RTC policy.
The environment RTC probe can be skipped for policy/fault cases. An owned-page
prelude runs before the WASM module, with dedicated fixture snapshot/reset/restore
operations; it does not alter device-wide browser policy or other tabs.

Pixel's first CDP connection failed even though Chrome was running. HTTP control
still delivered a working formal WASM page, and only its visible owned page was
closed. A later normal owned-page startup had working CDP on both devices:
Pixel Chrome `154.0.8037.126`, Y700 Chrome `154.0.8037.92`. This is a temporal
observation, not a static browser-brand limitation. Current receipts show exact
owned-URL CDP close, independent absence of owned forward/reverse rules, and a
closed local listener. Existing user tabs and the global ADB daemon were retained.

```sh
rtk proxy python3 -B scripts/android_pair_runtime.py \
  --output /tmp/miopunch-android-pair-browser-capability-new.json
```

The command is a browser load/control prerequisite, not a public member runner.
Each endpoint is selected by exact model and transport (`Pixel_6a` USB, `TB322FC`
network); serials and hardware identifiers are omitted. The final prereview
source additionally uses `reverse --no-rebind` and skips tab cleanup if the runner
never opened a page. Those last ownership guards were reviewed as source changes;
they were not relabelled as fresh device evidence.

## Formal native entry and build

[build_android_pair.py](../../../scripts/build_android_pair.py) builds selected
formal native binaries, optional JNI library and existing `MioPublicShell` from
the current Rust SDK. No `android_node_probe.rs` exists at this baseline. The
existing main entry is `miopunch-native-probe network session|maintain`; JNI
uses `src/android_shell.rs` and the shared desktop CLI. MIOPUNCH-73's native member
fixture and MIOPUNCH-79's dedicated member actor are selected with additional
`--bin` arguments; their product behavior belongs to their own source slices.

```sh
rtk proxy python3 -B scripts/build_android_pair.py \
  --ndk /mnt/c/Android/SDK/ndk/29.0.14206865 \
  --output /tmp/miopunch-android-pair-final-build-new \
  --target-dir /tmp/miopunch-android-pair-target-20261005 \
  --bin miopunch-native-probe --bin browser_wss_member_native \
  --features rtc-probe --jni-shell \
  --d8-jar /mnt/c/Android/SDK/build-tools/36.1.0/lib/d8.jar \
  --strip-deployment-debug
```

The first isolated build completed exit 0 in approximately 3m20s, using native
Linux Clang/Rust lld and the installed Windows NDK's target sysroot, without
executing Windows compilers. Concurrent source changes made its recorded
`qualifiedForFrozenRun=false`; its artifacts are provisional entry/toolchain
proof, not final new-wire qualification.

Y700's initial 195,795,296-byte debug artifact transfer hit the local 12s setup
wait and was force-terminated with actual exit -15. That first failure is retained;
no native process started in that failed attempt. Debug sections were stripped
only from owned deployment copies (originals retained), with both byte hashes and
exact transform exit in [deployment-initial.json](deployment-initial.json).
The approximately 110MB main/100MB fixture/101MB library copies deployed within
the unchanged control cap; no public or SDK deadline was extended.

On each device the formal main SDK created its own private node directory,
started a local session with empty auxiliary-resource replacement, returned
status Complete, and rejected an invalid local command. It then stopped exit 0,
kept the same material directory, and reopened through the formal JNI entry with
the original node identity matched as a boolean. Final exit was 0; exact owned
remote PID absence was observed immediately and later, readers ended, and the
owned directory was removed. No independent STUN, admission to a second peer,
member business, new protocol or physical selected data path was exercised there.

`NativeActor` supports file deployment/hash checks, private JSON-line or textual
stdin, event matching, same-directory stop/restart, and only its own PID/process
group cleanup. `close(remove_directory=False)` retains materials for the next
SDK phase; final close removes only its generated mode-0700 directory. It never
kills user Chrome, unrelated processes or the ADB daemon. TCP enhancement remains
subject to the existing Android/Linux platform guard.

## Receipts and source versions

- [pixel-wifi-connect.json](pixel-wifi-connect.json),
  [pixel-wifi-post-switch.json](pixel-wifi-post-switch.json), and
  [pixel-target-saved.json](pixel-target-saved.json): authorized switch, actual
  target connection/route and saved boolean, without credentials or serials.
- [pair-physical-prerequisites.json](pair-physical-prerequisites.json),
  [pixel-cloudflare-udp-comparison.json](pixel-cloudflare-udp-comparison.json),
  [pixel-google-udp-failure.json](pixel-google-udp-failure.json): finite observation
  successes and retained failure; no repeated observer scan after the instruction.
- [pixel-curl-first-failure.json](pixel-curl-first-failure.json),
  [pixel-java-tls.json](pixel-java-tls.json), [pixel-cdp-first-failure.json](pixel-cdp-first-failure.json):
  missing tool, normal system-TLS fallback and initial owned CDP failure.
- [browser-pixel-current.json](browser-pixel-current.json),
  [browser-y700-current.json](browser-y700-current.json),
  [browser-pixel-initial.json](browser-pixel-initial.json): actual formal-WASM
  load/control results and versioned cleanup measurements.
- [build-initial.json](build-initial.json), [deployment-initial.json](deployment-initial.json),
  [native-startup-initial.json](native-startup-initial.json),
  [native-y700-startup-deployment.json](native-y700-startup-deployment.json):
  provisional build, transformation and two-device SDK entry lifecycle.
- [helper-before-review-manifest.json](helper-before-review-manifest.json),
  [tooling-before-review](tooling-before-review), and
  [identities-before-review.json](identities-before-review.json): exact prereview
  source snapshots and evidence byte identities. Probe Java sources are preserved
  in [probe-sources](probe-sources); they must not be reused as product actors.

Each receipt identifies its actual executed version and scope. Later review fixes
are not retroactively applied to these historical results. Actual selected SDK
routes remain for the final frozen member run, with gate/new-wire/peer-match
booleans, fresh original-QUIC validation, business/replica results and its own
independent cleanup. Environment readiness does not close the ticket's data-path
qualification requirement.

## Concentrated helper review corrections

[tooling-after-review](tooling-after-review) and
[helper-after-review-local-validation.json](helper-after-review-local-validation.json)
preserve the first correction set: exact Chrome address-bar ownership, independent
SDK/tab/rule/listener cleanup, mandatory real cleanup facts, continued member
close/diagnostic/free attempts, complete build inputs and final source snapshot,
retained transformation failures, and preferred formal session diagnostics with
private material removed. Eight local Python tests and the Node VM checks passed;
these corrections were not rerun on devices or public services.

The final frozen version is recorded in
[helper-final-review-manifest.json](helper-final-review-manifest.json),
[tooling-final-review](tooling-final-review), and
[helper-final-review-local-validation.json](helper-final-review-local-validation.json).
Before the first formal SDK start it requires an observed absence of the exact
SDK IndexedDB on its owned local origin. Preexisting storage rejects start and
remains untouched. Stop/reopen retains the claimed store. Only final shutdown,
after successful SDK close/free and while holding the SDK's exclusive storage
lock, requests deletion of that exact claimed database. Actual success is awaited;
error, blocked, timeout, missing API or competing page fails cleanup. This is
local test-resource disposal, with no device-wide storage clearing.

The same version exposes the existing formal RTC-invalidation diagnostic getter
and samples its actual resource counters immediately and after 100ms following
network close, before free. Missing counters remain unknown; observed nonzero
counts fail cleanup. Local tests cover preexisting-store preservation, stop
retention, successful final deletion, blocked/error/lock failure, mandatory
storage receipt facts and a real late nonzero-counter branch. Historical device
results remain under their original hashes and do not prove these later branches.

The same reviewers identified two further Standards issues. Their narrowly
corrected version is frozen in
[helper-final2-review-manifest.json](helper-final2-review-manifest.json),
[tooling-final2-review](tooling-final2-review), and
[helper-final2-review-local-validation.json](helper-final2-review-local-validation.json).
A completed stop now retains its exact generation's close/free/error result.
Final shutdown summarizes all actual cleanup generations; a successful restart
does not erase an earlier failure or allow unproven closure to delete storage.
An IndexedDB blocked/timeout reply reports pending deletion and failed cleanup
within the controller cap, while its Web Lock callback keeps awaiting actual
success/error. Independent owned-tab termination remains observable. A pending
reply never qualifies graceful deletion or successful cleanup. Local VM
regressions reproduced both original failures and their terminal transitions;
the historical device/public evidence was not repeated or reclassified.
