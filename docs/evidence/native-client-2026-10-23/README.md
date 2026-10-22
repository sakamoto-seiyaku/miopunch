# Current native client: Windows/Linux qualification

2026-10-23 — MIOPUNCH-112. The actual current Windows PE ran on `laptop`
(Windows 11 Enterprise LTSC x64), while the Linux host ran under WSL2/Debian.
The [machine-readable summary](dual-summary.json) retains public IDs, saved
stages, pin/source hashes, actual selected business paths and process exits.
Invitations, private keys, admission secrets and business payloads are excluded.

| Acceptance | Result |
| --- | --- |
| Linux creates / Windows joins; Windows creates / Linux joins | Passed with distinct persistent node identities |
| Unapproved and authorization-only member business | Four actual public operation failures; no false completion |
| Manual request retry | Same applicant/request pair, one approval, unchanged authorization record count after completion retries |
| Interrupted completion | `PendingMaterials` survives stop/reopen; verified current secret completes the original attempt |
| Precode max-one retry | Saved use count remains one across interrupted completion, reopen and retries; authorization record count stays fixed |
| Business in both connection directions | 16 actual SDK PING/PONG exchanges, including reopened manual/precode members |
| Final identity and membership | Reopen retains the same node identity and complete membership; subsequent business succeeds |
| Owned processes | All 16 process instances acknowledged close and exited zero |

Focused public-input checks are recorded in
[failure-summary.json](failure-summary.json): invalid invitation fails before
creating state on both platforms; malformed JSON, unknown operations and an
oversized line each return `invalid_command`, preserve status and permit a
normal close/reopen. The final focused run also verified zero remaining native
processes at the exact owned Windows executable path, with no forced stop.

Raw two-device transcript and process stderr:
`/tmp/m112-dual-20261023-1/`. This run's private durable state remains only in
the owned local/remote temporary directories for diagnosis. Public report data
is selected from the transcript; bearer invitation files were removed.

The observed selected paths were IP paths with local Tailscale/VPN interfaces
and private peer addresses. SSH used the Tailscale laptop address for control.
No SSH tunnel carried member messages. This proves functional communication in
the available environment; it does not qualify VPN-free WAN hole punching,
every NAT combination, or the deferred RTC endpoint problem.

## Build provenance and preserved failures

Starting HEAD: `acac1d729a9e22919f0eaa87b81515c2edfb5284`, plus the preserved
starting worktree. Full starting snapshots and 452-input frozen graph are in
`/tmp/miopunch-m112-20261023/`; no blanket staging or source reset was used.
Rust 1.98.1, default features, committed dev profiles and locked offline sources
were used for both platform graphs. The initial batch budget was 8 GiB extra,
with the existing 96 GiB cache / 32 GiB artifact-store limits unchanged.

| Receipt | Result |
| --- | --- |
| `build-linux-1` | Preserved typecheck failure: optional precode count required explicit unavailable handling; 457 Fresh / 8 rebuilt |
| `build-linux-2` | Cargo success: 465 Fresh / 2 rebuilt; subsequent pin failed EXDEV between repository store and `/tmp` |
| `linux-pin-repair.json` | Verified source and exact built binary, pinned on the store filesystem under the lease; no recompilation |
| `build-windows-1` | Sandbox Docker access denied before Cargo; no cache-hit or build-success claim |
| `build-windows-2` | Cargo success: 198 Fresh / 330 rebuilt; first current Windows/default graph with stable image tooling; native system DLL imports recorded |

Windows compiler image:
`sha256:af5de852d8a8ae263b8eaa6278aea87c3fedc9030d7eb51a0d1b2613c9ce4c4f`.
Windows graph added 589,459,456 cache bytes. Rebuilt package/target inventory
and rationale are in `build-windows-2/rebuilt-review.json`. Each runtime case
reused the immutable qualified pins; no per-case compilation or cache cleanup
was performed.

The later driver review strengthened coverage enforcement to 502 local
source/asset/build inputs including workspace manifests, and added a
validation-only path. `driver-review-validation.json` retains rejection of an
omitted old input list before Cargo and acceptance of the expanded current
list without Cargo. The actual executable builds above used their original
452-input receipts; they are not relabelled as builds observed by the later
validator. Only Python/docs changed after qualification, so neither Rust graph
was rebuilt. The seven driver regressions cover omitted source/assets/member
manifest, newly added source, lock drift, cleanup isolation, result publication
and remote ownership.

The first sandboxed Linux smoke failed to bind a real socket. Its authorized
host-network retry created and closed normally. Local runner iterations 1/2
preserve failures from a wrong expected stage name, shared peer backoff and
interleaved SDK events; the final local admission/business suite passed at
`/tmp/m112-local-20261023-3/`. These were harness corrections; the real SDK
executables were reused throughout.

An affected cleanup/input rerun at `/tmp/m112-failures-20261023-2/` preserved
one Windows startup `PermissionDenied` (OS code 5) before `ready`. Its failed
verdict and cleanup result were published; no native process remained. A
same-parent directory create/rename/remove probe and three independent native
create/stop attempts with the same PE then passed, as did the final affected
input/reopen/cleanup run at `/tmp/m112-failures-20261023-3/`. The precise failing
filesystem operation and cause remain unknown; this is not claimed fixed.
No broad retry, ACL change or platform refactor was added to hide the failure.
