# MIOPUNCH-47 two-axis code review

Fixed point: `56d40e0ee07dee6a2ed0ec6a9115ed26c59be60d`.
Initial implementation: `2dde359b1ae7a5acf821c64cdf72bae210e3ea04`.
Final runtime and review correction: `f6817654881f364348794a5c5191fa2172a5fdb8`.
Independent initial and final reviewers used GPT-6.1 Sol HIGH Fast/priority.
Uncommitted user WIP and shared build defaults are excluded.

## Standards

Reviewed `56d40e0ee07dee6a2ed0ec6a9115ed26c59be60d...HEAD`, ending at
`2dde359b1ae7a5acf821c64cdf72bae210e3ea04` (one MIOPUNCH-47 commit).
Uncommitted user work and unrelated tickets are excluded.

**Documented-standard breaches: none found.** Sources: `AGENTS.md`,
`/home/js/.codex/RTK.md`, `CONTEXT.md`, current design entry points,
`docs/agents/domain.md`, `docs/agents/issue-tracker.md`, and accepted ADR 0005.
The integration note explicitly records policy choices; `vendor/iroh/UPSTREAM.md`
records fork compatibility. Restricted signaling remains separate from business
qualification. Evidence documents acceptance and limitations without becoming an
open-work ledger. Archived Go/OpenSpec gates do not apply. Tooling-enforced
format/type concerns are excluded.

**Heuristic judgements, both low priority and non-blocking:**

- **Possible Duplicated Code** —
  `vendor/iroh/src/socket/remote_map/remote_state.rs:2028–2061` repeats live-handle
  qualification and path projection in the peer and connection context branches:
  `".filter(|conn| conn.close_reason().is_none())"`,
  `".filter(move |(_, addr)| state.may_select(addr))"`, and
  `"PathSelectionData::live(...)"`.
  A shared connection-to-qualified-paths helper could keep eligibility changes
  consistent; retain the peer branch's distinct control-session exclusion.
- **Possible Duplicated Code** — `tests/path_maintenance.rs:464,700,796,1042,1125,1205`
  repeatedly constructs the owned signaling fixture:
  `"let broker = std::env::var(\"MIOPUNCH_MQTT_BROKER\")"`,
  `"pair.reconfigure(vec![Resource {"`, followed by MQTT custom-route lookup.
  A small fixture helper could centralize broker setup and route construction
  while keeping each case's faults and assertions explicit.

No additional baseline smells warrant a finding. Successful checks were not
rerun solely for review; unchanged slow NAT matrices remain skipped.

Standing user instructions retained and applied:

> 我说了，我允许对PLANE的修改。是我们必要的写入，下面所有的子代理都领取。

> 就目前这里的检查或者是那个单元测试，我们是不是有大量时间都耗费到了那个NAT相关的测试上面。我记得之前在MIO的时候，他那测试就非常费时间。我们有测完一次之后，只要没改这一块的代码就可以暂时先跳过NAT相关的测试了。否则每次都是特别浪费时间。

> 主线程只调度，子代理使用 技能实现和 handoff 怎么会是你要求的呢？

> 子代理使用 sol6.1 high 和 fast。

Result: 0 hard violations; 2 non-blocking smell judgements (Duplicated Code).

## Final correction supplement

Reviewed correction `2dde359b...f6817654881f364348794a5c5191fa2172a5fdb8`
and its integration with the full `56d40e0e...f6817654` MIOPUNCH-47 scope.
The original review and standing user instructions above remain applicable.

**Documented-standard breaches: none found.** The correction records its
fresh acknowledgment requirement in the integration note and all three affected
fork provenance notes, satisfying `AGENTS.md` and `docs/agents/domain.md` rules
to record choices explicitly rather than silently promote research suggestions.
It retains ADR 0005's restricted signaling and independently checked admission
boundary. The probe queues an existing QUIC PING and reads authenticated ACK
state rather than introducing a new cryptographic protocol.

Per-hunk review:

- `src/connection_recovery.rs`: `"path.start_probe()"` replaces stale snapshot
  success; `"self.check_repair_membership(original, control)?"` rechecks the live
  original and membership before returning acknowledged recovery.
- `vendor/noq/src/path.rs`, `vendor/noq-proto/src/connection/mod.rs`, and
  `vendor/iroh/.../path_watcher.rs`: the opaque `PathProbe` owns its original
  path and watermark. These narrow layer adapters preserve ownership and the
  established public seam; no new Speculative Generality or Middle Man finding
  is warranted.
- `tests/path_maintenance.rs`: the real shutdown fixture distinguishes a
  retained advertised address from actual delivery. Its repeated MQTT setup
  extends the initial low-priority duplication judgement; it is not an additional
  finding. The two original cleanup suggestions remain deferred.

No additional baseline smells warrant a finding. Passed checks were not rerun;
unchanged slow NAT matrices remain skipped under the user's explicit policy.
No runtime edits, staging, commits or tracker mutations were performed by this
review.

Final result: **0 hard violations; 2 existing non-blocking smell judgements**.

The implementation retains the two low-priority duplication judgements as
non-blocking review observations. Neither represents a documented requirement
breach, and unrelated cleanup is deferred under the user's completion priority.

## Spec

Reviewed committed `56d40e0ee07dee6a2ed0ec6a9115ed26c59be60d...2dde359b1ae7a5acf821c64cdf72bae210e3ea04`, using the captured MIOPUNCH-47 and MIOPUNCH-14 specifications, current domain notes and ADR-0005. Protected user WIP is excluded.

**One finding — P2, implemented requirement with an incorrect success condition.**

The ticket requires “分别验证已有备用路径、无预建备用但经控制入口在原连接存活窗口内协调新路径、原连接已关闭三种情况”; the parent says “如果原连接最终超时、关闭或无法维持，向调用方明确报告失败，不伪装成仍可传输。”

In [src/connection_recovery.rs:187](../../../src/connection_recovery.rs#L187), `validate_connection_candidates` reports success whenever an offered address occurs in `original.paths()`. For an unchanged-address physical outage, the previously established path can remain listed during the original QUIC connection's timeout window. The peer's offer can repeat that address. [open_path_on_conn](../../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1581) immediately accepts an already-open address without fresh validation, so recovery can return `Ok(ConnectionPathRecovery)` while the only business path still cannot carry bytes. The close check distinguishes a terminated connection but cannot prove recovery on a locally living one.

Require bounded fresh confirmation on the original data path, or distinguish newly established paths from pre-attempt entries and verify any reused entry before reporting success. Add a real unchanged-address physical-loss case with live control; it should remain retryable while the original lives and become terminal on its actual close, rather than report recovery solely from the old list entry.

The final thirteen-case evidence remains valid for its declared Linux IPv4 loopback matrix. Its no-backup proxy cases offer a different reachable endpoint address, so they do not exercise this branch. No other missing/partial requirement or unasked behavior was found: RTT policy, budgeted reevaluation, explicit resource restrictions, per-connection eligibility, two-sided protected repair and new-identity failure evidence fit the ticket. Broader NAT/platform support is explicitly unclaimed and is not treated as missing acceptance.

Review used source and recorded evidence; no tests were rerun and no runtime/test files were edited.

## Final supplement — f6817654881f364348794a5c5191fa2172a5fdb8

Reviewed correction `2dde359b...f6817654` and the resulting MIOPUNCH-47 implementation against the same ticket and parent requirements. **The P2 finding above is resolved; zero remaining Spec findings.**

`validate_connection_candidates` now starts one fresh probe for each qualified offered path and requires its acknowledgment before returning success. `noq::PathProbe` retains the exact original connection/path. The protocol captures the next unused data packet number while queuing PING under the same connection lock; earlier acknowledgments cannot satisfy that watermark. The existing authenticated, per-path ACK processing supplies the observation. Abandoned paths and connections outside the established state reject both probe creation and acknowledgment checks, including before packet draining completes. Current membership and original close checks remain decisive.

The real [same-address RED](same-address-red/control_paths_do_not_report_stale_same_address_as_recovered.log) demonstrates the original false `Ok`. The [final fourteen-case matrix](final-fresh-ack-acceptance/results.json) passes healthy unchanged-address confirmation, physical loss returning Retryable while the original lives, and Terminal after its actual close, alongside the prior same-stream, identity, quality, restriction and isolation cases. [Protocol checks](fresh-ack-protocol-checks/results.json) and [immediate-close evidence](fresh-ack-immediate-close-check/run.log) cover old ACK rejection, distinct paths, abandonment and closed connections; final library/type/WASM/format receipts pass.

No new missing/partial requirement, scope creep or incorrect success condition was found. The probe uses ordinary QUIC traffic and existing ACK handling; it adds neither a new wire handshake nor application recovery. Declared Linux loopback qualification and compilation-only browser evidence remain accurately bounded.

This supplement inspected source and recorded results only. No successful checks or unchanged slow NAT matrices were rerun; no runtime/test files were edited. STOP.

Standards: 0 hard violations, 2 non-blocking duplication judgements; Spec:
initial 1 P2, fixed with genuine same-address RED/GREEN, final 0 remaining.
