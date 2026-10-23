# File-transfer module qualification

2026-10-23 — [MIOPUNCH-114](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/918b2f20-b668-4a72-93da-f5dc5419ea29).
See the [operation guide](../../notes/2026-10-23-file-transfer.md) and
[machine index](summary.json). Secret-free raw receipts, controller sources,
build sources and original failures remain under the ignored repository
`.artifacts/runs/` paths below. Private node states, invitations, recovery
cards and browser profiles are excluded from this report and Git.

## What ran

Public `NativeClient`/`miopunch-client` operations ran on the actual Linux
host and Windows 11 Enterprise LTSC x64 laptop (PowerShell 5.1). Actual
Chromium 153 profiles ran the compiled `BrowserNetwork` WASM with independent
durable identities and IndexedDB/OPFS storage. Browser tests used real
source `File` objects and read back actual saved files for independent hashes.
The native endpoints generated and hashed their files on their own OS.

SSH deployed the Windows executable and carried control, private invitations
and evidence. File payloads travelled through admitted SDK connections.
Native pairs selected available IP and public-relay paths; browser pairs used
the configured public relay. These results qualify the available VPN/IP and relay environment,
without claiming unrestricted WAN/NAT coverage.

All receipt paths in this table are relative to
`.artifacts/runs/m114-max-20261023/`.

| Receipt | Observed result |
| --- | --- |
| `native-final-full-8/` | Current Native8, both creator roles and file directions: 20 saved deliveries, including four 256 MiB transfers interrupted and continued from a nonzero offset. Empty, 11-byte, 4 MiB and 64 MiB files also matched independent destination SHA256. Premembership, disabled service, source/save failures and cancellation were exercised. |
| `native-bounds-8/` | Current Native8, four directions across Windows/Linux: four concurrent 64 MiB tasks completed per direction, a fifth was rejected, and cross-network peers created neither a receiver task nor a saved file. Sixteen actual saved hashes matched. |
| `browser-functional-8/` | Current Native8/WASM8, both creator roles: 20 bidirectional saved deliveries, four interrupted 16 MiB continuations, ordinary receiver cancellation and saved-file/identity persistence after reopening. Empty, 11-byte, 4 MiB and 16 MiB files matched independent saved hashes. This run invoked public operations without held-storage or injected-error hooks. |
| `browser-final-full-4/` | Native6/WASM4, Native creator and browser creator roles: 20 bidirectional saved deliveries, including four interrupted 16 MiB continuations. Also covered empty/11-byte/4 MiB/16 MiB files, actual OPFS save failure, both-party cancellation during held publication, held-abort cleanup, and saved-file/identity persistence after reopening. |
| `browser-final-seams-4/` | Native6/WASM4: actual Browser↔Browser 64 KiB duplex streams; 65 KiB write and a second pending read rejected; stream cancellation followed by another stream on the same admitted connection. Four concurrent 16 MiB Browser→Native files completed, a fifth was rejected. Two independent browsers also exchanged and saved 4 MiB files in both directions. |
| `retained-green-6-4-linux/`, `retained-green-6-4-windows/` | Native6/WASM4 with each actual native OS: disconnect the receiver, stop the sender so it cannot retry, then cancel or disable an inactive retained partial. Each adapter completed two cancel and two disable cycles, removed every partial and released all four slots. A fifth transfer on each adapter then saved and hashed successfully. |
| `expiry-8-7-linux/`, `expiry-8-7-windows/` | Current Native8 on each actual native OS, and earlier WASM7 OPFS: disconnect the receiver and stop the sender. Each retained partial expired without a peer retry, was actually removed before `failed` / `interrupted`, and released its slot. A subsequent 11-byte transfer saved and matched its independent hash on every receiver. |
| `retained-browser-8/` | Current WASM8 with unchanged Native8: without a peer retry, two cancel and two disable cycles removed four actual inactive OPFS partials and released all slots. The fifth actual saved file matched its independent hash. |
| `expiry-browser-8/` | Current WASM8: the inactive OPFS partial expired without a peer retry, was removed before `failed` / `interrupted`, and the next 11-byte file saved with the expected independent hash. |
| `browser-idle-green-4/` | WASM4 with the unchanged Native5 counterpart: after a real 57.5-second idle, the first 11-byte transfer saved at 68.93 seconds and a fresh second transfer saved at 82.70 seconds. Both saved hashes matched; the shared incoming dispatcher remained live. |
| `browser-foreground-clock-5/` | Earlier WASM5: an explicit 100 ms foreground accept deadline expired at 106.7/104.5 ms with the file service closed/enabled. Registrations were removed and the dispatcher remained live. |
| `browser-formatting-smoke-5/` | Earlier WASM5 with Native6: eight actual bidirectional empty/11-byte saved deliveries across both creator roles, saved-file reopen checks, save-abort failure and both-party held-publication cancellation. All hashes and cleanup checks passed. |
| `final-checks-8-7/` | Earlier check: twelve owned Rust files passed rustfmt, four Python controllers parsed, two JS modules passed syntax checks, and the Native8/WASM7 inputs matched that graph. All 38,553 unowned starting files matched their original hashes. |
| `final-checks-8-8/` | Current check: twelve owned Rust files pass rustfmt, four Python controllers parse, all three owned JS modules/tests pass syntax checks, diff check passes, and Native8/WASM8's 511/534 inputs match. All 38,553 unowned starting files remain unchanged. |

Every completed formal runtime receipt above reports no controller-source
drift. Native processes exited zero; Windows cleanup reported zero forced and
zero remaining owned processes. Browser controllers confirmed closed contexts,
ended readers and absent owned process groups. Owned private runtime
directories were removed. No firewall, routes, VPN settings or unrelated
processes were changed.

The explicit raw data-stream test was **Browser↔Browser**. Shared Native
`DataStream` framing also ran inside actual Native↔Native and Native↔WASM
file protocols; a separate raw Native↔Browser stream experiment is not claimed.

The three current FileSink storage regressions are in
`.artifacts/runs/m114-storage-tests-8/`. They use an owned adapter and ordinary
filesystem operations: cancellation remains pending until cleanup completes;
an actual cleanup error during preparation or retained expiry produces
`SaveFailed` with no successful receipt. Both production adapters call the
same preparation-rejection helper. These are adapter tests, not a claim that
the current Chromium run reproduced an exceptional OPFS removal failure.
All three passed and removed their owned test directories. The test consumer
reused verified Native8 Cargo libraries, compiled only its own test executable
under the shared lease and pinned it once, without Cargo or lock resolution.
Its receipt writer failed after compilation/pinning while serializing a
`Path`; that controller failure and raw compile receipt remain. Evidence was
finalized from the same verified pin, then its tests ran once without rebuilding.

The exported JavaScript storage adapter has six further ordinary filesystem
regressions in `js-final-storage-8/`, with the committed test in
`browser-probe/harness/file-storage.test.mjs`. Four sequential failing cases
were fixed and rerun: failed removal after publication, a completed JavaScript
save not yet observed by Rust, a writer-abort error, and failure opening the
fallback destination writer after its file had been created. The final suite
also checks actual fallback contents and preservation of an existing destination.
All six passed with source drift and owned temporary-directory residue empty.
Deliberate error-case residue was removed by the test's own cleanup, not reported
as SDK cleanup. These qualify the exported adapter seam with an owned filesystem
backend; they do not claim current Chromium exceptional-removal qualification.
The original Node isolated-runner trace exposed only an outer failure; the
separately retained direct Node test records contain the actual failed assertions.

## Artifact and source provenance

Build drivers used the one shared `target/` cache under its lease, Rust
1.98.1, locked/offline dependencies, the committed dev profile, existing
features and stable Windows compiler wrappers. No cache cleanup occurred.
`scripts/pin_artifact.py` pinned selected outputs into the shared immutable
store, independently of mutable Cargo output. Runtime drivers never compiled.

| Artifact | Original build receipt | SHA256 | Cargo Fresh / rebuilt |
| --- | --- | --- | --- |
| Current Linux client | `.artifacts/runs/m114-linux-file-8/build.json` | `52e53626c63af6d02e938ee424439afb994a6d4727bc7f3dc01deff414f82388` | 465 / 2 own targets |
| Current Windows client | `.artifacts/runs/m114-windows-file-8/build.json` | `2e63b29f97a5fa384c86fa1ddc981674f416852491bbbbbf90095025f50a84af` | 526 / 2 own targets |
| Current WASM8 package | `.artifacts/runs/m114-wasm-file-8/build.json` | `b8d02c708799cf2e8a2b55e231515e965eea5c54f47f0af3361e8f78a34594f7` (package) | 295 / 1 own target |
| WASM4 tested full package | `.artifacts/runs/m114-wasm-file-4/build.json` | `6000661eae6fab66dd5da96cd773107eb93c9702e0f0683a0839344a8c8a5f60` (package) | 295 / 1 own target |
| Earlier WASM5 formatted package | `.artifacts/runs/m114-wasm-file-5/build.json` | `4cc51a562c6d21916237c01f3e748a4a47ee110f24392d0f54c317512bd8420b` (package) | 295 / 1 own target |

The earlier WASM4 and WASM5 differed only in rustfmt's ordering of one module declaration and
wrapping one constructor call in two owned source files. Native6 was reused.
Their executable hashes are respectively
`7d041557ba694f1314200cfcb7be52a07e86c4784eac421b021458aab1c29b04`
and `6af81572d4f49b49fcdb3e86fa696ac9948414aa885784cb8a090a5dec1b478c`.
The binaries are different; full WASM4 results are labelled with that revision.
Build receipts retain source archives, selected depfiles, input hashes and
the exact bindgen tool hash. The final formatting graph was declared before
Cargo; only its own WASM target rebuilt. Later Native7/WASM6 implemented R2
creation ownership and cleanup-error fixes. Current Native8/WASM7 add shared
preparation rejection, task reservation before Native file creation and bounded
receiver connection ownership through shutdown. WASM8 changes the JavaScript
adapter's output/writer ownership and truthful cleanup errors. Its executable
hash remains identical to WASM7:
`f0a1e99b18a025fcd2e79802c2a50051928df97785e4a92c37d633afea22f1b1`.
Their packages differ: the current packaged `file_storage.js` SHA256 is
`99ede556b824fd01dcede44f537b83269e33e636ad884c371a5e0d7fe707b1a7`.
Each changed graph was declared before its one build; unchanged dependencies
remained Fresh. Store/cache/batch budgets remained 32/96/8 GiB respectively.

## Retained failures and review fixes

The highest public operation first failed on the accepted predecessor because
`files_enable` did not exist (`red-native.json`). Subsequent actual failures
are preserved rather than overwritten:

- `native-bounds-red-2/`: concurrent setup exceeded the existing per-peer
  pending-contact contract. The Native adapter now serializes contact setup
  while connected file streams run concurrently.
- `browser-cancel-red-1/`: a terminal cancellation preceded the actual OPFS
  abort. Platform cleanup now owns noncancelable I/O before reporting terminal.
- `browser-publish-red-2/`: an in-flight move published a file after early
  cancellation. `browser-publish-green-3/` and the full WASM4 suite hold the
  real move promise, cancel from each party, then verify removal after release.
- `browser-final-full-3/`: the continuation setup did not reach a receiver and
  exhausted the controller deadline. `browser-idle-red-3/` isolated the cause:
  a background wait consumed the new connection's 60-second handshake budget.
  A first transfer after 57.5 seconds idle failed at 60.47 seconds without a
  receiver task; the same identity, file and permission saved on a fresh retry.
  Service clocks now start at the actual incoming packet; explicit callers
  retain their original absolute deadline. Full WASM4 continuation passed.
- `retained-red-5-3/`: old Native5 and WASM3 both retained an interrupted partial
  forever after cancellation/disable when the sender could not retry. The
  shared receive/cleanup transitions now have a tracked owner that completes
  actual cleanup and releases the task slot. Both actual native OS and WASM
  regression receipts above passed.
- `native-create-close-red-6/`: closing the old host could outlive an in-progress
  file creation and leave a late partial. Native7's completed paired GREEN and
  full suite retained an owner through that creation. Current Native8 also
  closes its bounded receiver connections during shutdown, reserves task state
  before creation, and rejects new ownership after host close/drop.
- `browser-create-abort-red-5b/`: old WASM5 hid an actual preparation cleanup
  failure as `Cancelled`. The technical host explicitly removed that injected
  residue; this was not SDK cleanup. The corrected shared helper reports
  `SaveFailed`, qualified by the current owned-storage regressions above.
  These completed fault fixtures are historical evidence and were not rerun
  during the ordinary continuation.

Independent R1 Spec review reported the retained-partial ownership and idle
clock defects; Standards reported no documented breach and a substantive
duplication heuristic in that same receive lifecycle. Both were accepted.
`review-problem-classes.json` preserves the original 11:27:31 UTC fix clock,
the targeted closure at 11:56, and the broader reopened investigation triggered
at 12:21:47. The shared implementation, caller clocks, active/inactive cleanup,
disable/close and related regressions were broadened without resetting the
clock. R2 independently closed both R1 correctness findings and the duplication,
then identified the creation-ownership and preparation-cleanup findings above.
The broader audit continued through preparation, I/O, publication, retention,
expiry, disable, close and drop without resetting its clock or review rounds.
Review reports are retained with their exact reviewed source snapshots.

R3 independently closed both exact R2 findings, while Spec found one remaining
publication branch: JavaScript discard suppressed a rejected save promise when
cancellation-time output removal failed. The same class therefore remained open
at round three, triggering further expansion at 14:34:13 UTC. The ordinary Node
regressions above confirmed that case and expanded the audit to writer abort,
provisional fallback output and save-result handoff. Discard now retains those
owners, observes failures, attempts remaining cleanup, and retracts an output
whose save success has not been handed to the SDK. Earlier class clocks remain.

Early development controllers were edited while runs were active, and some
old result hashes described the controller at completion. Those batches are
explicitly development traces in `controller-provenance-audit.json`; their
raw observations and immutable SDK provenance remain. No missing original
controller was reconstructed as consumed-source evidence. Formal runs snapshot
their actual imported Python and Node sources at startup and verify no drift
after cleanup.

A separate four-by-64 MiB browser queue experiment made progress but exceeded
its controller's 240-second case budget. It is retained as a controller timeout,
with cleanup; it establishes neither an SDK size limit nor a transfer timeout
defect. The bounded queue was formally checked with four 16 MiB files instead.
The incorrectly assumed empty admission history in an early stream fixture was
also corrected to select the actual new applicant.

## Limits

This module enables a live local receiver for approved members. Service ACLs,
scoped guest invitations and guest receipt confirmation are MIOPUNCH-115.
There is no formal UI, Android lifecycle qualification, persistent transfer
recovery after process loss, VPN-free NAT qualification or M111 RTC fix here.
The historical isolated Windows startup OS5 root cause remains unclaimed.
