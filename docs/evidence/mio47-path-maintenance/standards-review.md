# MIOPUNCH-47 Standards review

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
