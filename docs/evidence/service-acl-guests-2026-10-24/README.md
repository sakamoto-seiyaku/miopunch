# Service ACL and guest qualification

[MIOPUNCH-115](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/9dbf4901-1310-49d0-bb20-2d5dbb5994ff)
is qualified for its authorized non-UI scope. The
[operation guide](../../notes/2026-10-24-service-acl-and-guests.md) describes the
public Native/WASM interfaces. [summary.json](summary.json) records exact receipt
hashes, source graphs, current results and revision-labelled prior breadth.
Raw logs, frozen controllers, original failures and immutable SDK pins remain
under `.artifacts/runs/`.

## Source graphs and current acceptance

All graphs use Rust 1.98.1, locked dependencies, committed profiles, the shared
Cargo cache and immutable artifact store. Runtime cases reuse these pins.

| Graph | Receipt | Fresh / rebuilt | Artifact SHA256 prefix |
| --- | --- | --- | --- |
| Linux Native11 | `m115-linux-services-11` | 465 / 2 own | `894cafb366c90651` |
| Windows Native11 | `m115-windows-services-11` | 526 / 2 own | `10d9be6776355892` |
| WASM6 | `m115-wasm-services-6` | 295 / 1 own | package `a487fe51c1c721b6` |

The Native and WASM manifests still match all 514 and 538 declared inputs.
Standalone consumers verify the pinned SDK, complete source archive, lock and
selected libraries; they invoke no Cargo and rebuild no dependencies. The
original 8 GiB batch, 96 GiB cache and 32 GiB artifact-store budgets remain in
force. No cache cleanup or profile change was needed.

| Current result | Actual qualification | Receipt |
| --- | --- | --- |
| Shared policy and storage | 26/26 public/exact-source assertions, including the real-filesystem stale-receipt negative | `m115-public-boundaries-11` |
| Windows/Linux files | 20 independently hashed saves, four continuations, live cancellation and genuine save failures; both OS roles | `m115-native-file-regression-11` |
| Windows/Linux guests | Six independently hashed saves; one-identity binding, scope denial, pending without sink, rejection, approval, reconnect without extension, exit and remote sharing restriction | `m115-native-guests-11` |
| Native StrongRemove | Two actual live transfers stopped at 2,555,904 and 2,490,368 bytes; baseline/post-correction saves passed | `m115-host-correction-retention-11` |
| Completed-cache retirement | 66 actual saved hashes; new receipt and original saved file survive disable; `retainedCacheNegativeObserved=false` | `m115-retirement-assertions-11b` |
| Native live policy stops | Both OS roles for ACL and received guest revocation; both directions for creation-time expiry; no receipt/saved/partial file, receiver remains enabled, subsequent save hash passes | Lifecycle receipts described below |
| Native/WASM files | 20 actual saved hashes, four continuations, persistence/reopen and cancellation through the ordinary functional host | `m115-browser-file-regression-11-6` |
| Native/WASM services | Seven actual saved hashes, Native/browser administrator parity, independent WASM guests and OPFS approval/rejection/persistence | `m115-browser-guests-11-6` |
| WASM StrongRemove | Two actual OPFS transfers stop and clean; four independent baseline/post-correction saved hashes; verified management commits even when accompanying obsolete policy is rejected | `m115-browser-correction-11-6` |

Completed platform runs closed their owned SDK processes/profiles, removed
private runtime directories and reported no controller source drift. Windows
owned-process checks reported zero forced terminations and zero remaining
processes. Chromium reader and process-group checks passed. Actual selected
IP/VPN/Relay paths remain in each file task. SSH carries deployment, control,
private invitations and evidence; source files are generated on their sending
OS and business bytes travel through miopunch.

## Exact lifecycle and failure provenance

`m115-native-lifecycle-11` remains **overall false**. It completed four actual
ACL/revocation stops and the Linux-guest → Windows-provider expiry at 94,699,520
bytes, followed by an independently hashed ordinary save. Its retained raw
clock/event audit proves receiver progress → deadline crossing → sender still
transferring → first terminal observation. Later directory and service-status
checks cannot extend that witness.

The other expiry did not start: a measured 4.674-second scheduling pause crossed
the fixture's five-second approval window. The larger-source follow-up
`m115-native-expiry-11b` also remains false: the controller's 45-second
preparation observation ended before the offer became ready at 46.14 seconds.
These are unmet fixture prerequisites, not observed product expiry failures.
The complete timing audit distinguishes source hashing, controller observation,
the SDK's subsequent 60-second confirmation wait, endpoint clock offsets and
terminal/cleanup observations.

Only the missing direction was then run in `m115-native-expiry-11c`, using the
same Native11 pins: an explicit 180-second grant, 1 GiB source, delayed send,
twelve-second approval lead and 90-second preparation observation. It passed
real progress, expiry stop at 582,549,504 bytes, no receipt/saved/partial file,
unchanged receiver availability, the source's public expired status, local
rejection and a subsequent independently hashed save. No SDK clock or product
time policy was changed. Together these retained case observations qualify all
six current Native policy/expiry stops without relabelling the earlier whole
run as passed.

The first public file/ACL chain preceded static review. M114's retained binary
first saved a real file, then rejected the absent service operation. The changed
SDK exposed a signed-enum serialization defect in `m115-acl-local-2`; the same
real file/ACL assertions passed on Linux3 after correction. Linux5 guest and
Linux7 live-stop results, and Native7/WASM2's seven-save early smoke, remain
revision-labelled evidence.

R1/R2 public failures and actual Native9/WASM4 StrongRemove failures are retained.
The latter kept an obsolete management view while real file bytes continued.
Current Native11/WASM6 cases verify durable correction and physical stop. The
completed-context storage RED retains 25 passes and one failure; its identical
negative now passes within the 26-case consumer. The two earlier 65-delivery
metadata probes had already retired their cache and observed legitimate fresh
saves. They are not counted as product RED or retained-cache rejection. The
corrected consumer requires explicit `FileError::Rejected` from stream and
receiver to claim that negative branch.

Older timing failures and the browser's cancelled-versus-failed assertion are
also retained. Native10's lifecycle physically stopped all six transfers but
failed a later controller-clock assertion; an independent sampler measured
realtime steps of about ±16.66 seconds and endpoint clock offset. An early
Native10 Windows startup `PermissionDenied` occurred before ready/business
assertions. The same pin passed after owned-path diagnosis and serialization;
its exact original OS cause is not established.

## Review and scope

Independent R3 Standards/Spec reviews and focused assertion checks have zero
remaining findings. They reviewed code and assertions together, verified all
34 composed source hashes, and preserved the original problem-class clocks and
whole-class expansion. Production did not change after R3; the final changes
corrected assertion boundaries and preparation observation. Final documentation
records actual results without restarting unchanged code review.

Native10's ten-save full ACL and four-save admission results, and Native10/WASM5's
advanced ACL/revocation/expiry/active-exit result, retain their exact prior graph
qualification. The final graph's ordinary, guest, correction and Native lifecycle
cases cover the changed ownership/receipt paths; earlier results are not silently
promoted to a new source revision.

This establishes the selected Windows/Linux and independent Chromium scope.
It does not establish unrestricted WAN/NAT, Android lifecycle, formal product UI
or the deferred M111 RTC diagnosis. Starting unrelated work and AGENTS.md's
separate user-authorized efficiency change remain outside the M115 commit.
