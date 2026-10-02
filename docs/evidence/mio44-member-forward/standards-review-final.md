# Standards follow-up — MIOPUNCH-44

Reviewed only `b0629e98ab167f42d9e34d2f901e3845ae212da9...eb2875f6e956c70ed58270d1aa6297c896b2b04c` in `src/native_network.rs` and `src/member_transport.rs`, using the code-review skill's Standards axis. The original report remains unchanged.

Both accepted suggestions are resolved:

- `src/native_network.rs:499`: `automatic_mqtt` becomes `configured_startup` consistently at its declaration and both branches. The name reflects the configured assembly that installs MQTT resources and the shared member/MQTT bootstrap coordinator. The original Mysterious Name concern is closed; this is a naming change without changed branching behavior.
- `src/member_transport.rs:105`: `transmit_packets` owns the formerly repeated queue-draining loop. Both forwarding and attachment call it. The helper still performs `check_member(gate, connection)?` for each dequeued packet immediately before `write_packet(send, packet).await?`, propagates both errors, and returns successfully when the queue closes. Their separate receive loops, shutdown selection and lifetime timeout remain local. The original Duplicated Code concern is closed.

No new documented-standard violation or baseline smell warrants a finding in these hunks. The changes preserve the current-baseline/domain rules in `AGENTS.md` and `docs/agents/domain.md`, including ADR 0005's independent membership boundary and restricted signaling role.

Recorded verification: `review-refactor/results.json` reports `passed: true` with all nine ordered real member checks exiting 0; `all-targets.log` records successful compilation and `format.log` is empty. This follow-up did not rerun tests or unchanged slow NAT matrices and does not supply a Spec verdict.

Final Standards status: 0 hard violations, 0 open judgment-call findings; both original suggestions resolved.
