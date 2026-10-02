# MIOPUNCH-52 desktop management evidence

The final rerunnable entry is `scripts/check_desktop_windows.py --output NEW_DIR`.
[The delivery note](../../notes/2026-10-03-desktop-management.md) describes the
CLI, trust boundaries, protocol selection, persistence adapter and exact
Windows qualification limits.

## Runtime acceptance

[platform/results.json](platform/results.json) records all 11 scenarios passed:
a raw Windows-to-Linux QUIC round trip, then both Linux/Windows directions for
creation/manual joining, pending-material restart, daily/genesis management,
precode restore/count handling, and resource restrictions/unreachability.
[platform/processes.log](platform/processes.log) contains the actual public
CLI outputs, never invitation capabilities or private material.

The run built both artifacts and isolated tool images from the committed
Docker recipe. [commands.json](platform/commands.json) records every actual
command exit; [cleanup.json](platform/cleanup.json) records removal of both
owned containers. Rust, MinGW and Wine versions and build logs are adjacent.
This is a real Windows PE executable under Wine 10, using real signatures,
Windows random-number APIs, TLS/QUIC and storage. It is not a native Windows
kernel/NTFS/ACL or physical-power-loss certification.

## Affected checks and first failures

[validation.json](validation.json) summarizes the checks. Seven native suites
passed 105 entries; the corrected full precode suite passed 14, and two focused
resource persistence tests passed, for 121 reported entries. That total
includes five inert crash-worker entry points; their parent tests perform the
real subprocess interruption scenarios. No unchanged slow NAT matrix was run.
Native all-target typechecking, WASM typechecking and formatting passed.

The original combined native command exited 1: the precode expiry fixture's
first redemption could miss a two-second wall-clock window that included
creating the applicant and establishing TLS. Its isolated run passed. The
fixture now establishes those real prerequisites before starting the deadline;
the unchanged expiry policy is then checked in the complete 14-test parallel
suite. Both the original failure and the passing checks are retained under
[regressions](regressions/), rather than reporting the original command as
successful. The other seven passing suites were not rerun merely for that
fixture-only correction.

[first-failures](first-failures/) preserves the missing-CLI REDs, actual Windows
directory-access failure, and unused-DLL PE export failures. Wine 8's loader
lacked `bcryptprimitives.dll`; the controlled runner uses Wine 10 without a
crypto shim. The Windows filesystem correction retains Unix behavior; the
vendor changes remove unused cdylib artifacts only. All 32 upstream Rust
files in the newly vendored locked iroh-relay package were compared unchanged.

[Independent two-axis review](code-review.md): Standards found no hard
violations and two optional duplication heuristics; Spec found no required or
optional findings. Optional maintenance suggestions remain parked under the
user's completion priority. The reviewers inspected the saved evidence without
rerunning tests.
