# MIOPUNCH-54 migration evidence

Delivery, persistence decisions, public SDK entry and qualification boundaries:
[network migration note](../../notes/2026-10-03-network-migration.md).
Reproduce with `rtk proxy python3 scripts/check_network_migration.py --output /tmp/mio54-run`.

Final migration checks ran in ticket order in `/tmp/mio54-focused-final-v2`:
`migration-first.log`, `migration-priority.log`, `migration-remaining.log` and
`migration-results.json` preserve all three exit0 results. Six scenarios pass;
the seventh reported test is the inert `migration_crash_worker`, driven by the
parent's eight actual exit77 subprocesses. The checks cover real source member
traffic during preparation and its termination at switch; complete destination
materials plus actual member LogSync; genuine MQTT signaling with no data path;
source-exit progress rollback; same-key two-network history and independent
permissions; offline per-device confirmation and expired/fresh invitations;
and explicit cancellation with interrupted terminal-marker recovery.

`first-red.log`, `restart-red.log` and `cancel-red.log` retain missing-entry/hook
RED checks. `intermediate-compile-error.log` is an actual failed intermediate
syntax check, followed by successful final receipts, not a passing runtime run.
The initial successful source/data and priority scenarios preceded broader
checks. Necessary fixture corrections retained real connection handles until
the caller finished and synchronized the third destination peer's new member
view before asking it to connect.

`affected.log` reports 94 passes across eight affected suites: admission
completion20, gate18, daily management6, manual admission14, native network5,
offline replica12, precode14, revocation5. Four entries are inert worker entry
points whose parent tests run real process faults. The real MQTT environment
was supplied, so its environment-gated checks executed. `results.json` contains
the actual command/exit receipts, including native all-target, WASM and fmt
exit0. A later narrow directory-flush completion and cancellation-resume check
were followed by the final ordered migration run, plus native-network5,
all-target and fmt checks in `final-*.log` / `final-results.json`. The shared
browser code did not change after its compile check. Existing unrelated
`unused_mut` warning in `tests/tcp_punch.rs` remains.

`cleanup.json` and `migration-cleanup.json` record successful removal of each
runner's owned container and volumes. The separate exploratory `mio54-mqtt`
container was also removed. No other containers, device resources or shared
build caches were changed. Private test fixtures remained under OS temporary
directories and are not included here. Unchanged slow NAT suites were skipped
under the user's standing instruction. No platform runtime claims beyond the
native Linux migration SDK are inferred from earlier tickets.

Independent [Standards](standards-review.md) and [Spec](spec-review.md) reviews
compare implementation `c85fb62ca4d56db27ff93f1351179a385be88f7c` against baseline
`f7d298116cd53cb16d92729fd1aec18467dffbd4` and the current Plane ticket/parent.
[Separate-axis aggregation](review.md): Standards 0 hard findings / 1 nonblocking
membership-query duplication heuristic; Spec 0 findings. The optional refactor
is parked under the user's completion priority. Both reviewers actually ran
Astra/low/priority before the timed cutoff, independently confirmed by root.
No reviewer reran suites. Raw terminal logs retain trailing blank EOF lines;
`git diff --check` flags those artifacts, while source fmt checks pass.
