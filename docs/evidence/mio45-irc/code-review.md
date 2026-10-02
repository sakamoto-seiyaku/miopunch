# MIOPUNCH-45 two-axis review

Fixed point: `2b3711ab5a9af39a560902f52bcf8f6f46cd65b9`. Initial implementation: `4c4b5dc387a56f9d968d2d49cf33cb098052131e`. Accepted maintenance cleanup: `b4ec3c29fbf178fcb8b2564f22eededdb3820578`. Both axes were independent GPT-6.1 Sol HIGH Fast/priority agents.

## Standards

# MIOPUNCH-45 Standards review

Reviewed committed diff `2b3711ab5a9af39a560902f52bcf8f6f46cd65b9...4c4b5dc387a56f9d968d2d49cf33cb098052131e` (one commit: `Implement MIOPUNCH-45 bounded IRC control bootstrap`). Working-tree edits were excluded.

**Hard documented-standard violations: none found.** Sources: root `AGENTS.md`, `CONTEXT.md`, `docs/agents/domain.md`, current design entry points, ADRs 0004/0005. The change preserves the restricted control-bootstrap vocabulary, independent membership authorization and stable node identity. Adapter choices and qualification limits are recorded explicitly; no archived Go/OpenSpec gate or research suggestion becomes a new product requirement. No separate coding-standard file exists. Tooling-enforced formatting/compilation is excluded from this axis.

**Non-blocking judgement calls:**

1. Possible **Duplicated Code**, `src/resources.rs:156–171`: the new `irc_resources` copies `mqtt_resources`' parsing, permission filtering and origin deduplication, differing only in scheme. Quoted hunk: `if url.scheme() == "irc" && self.allows(&resource.url, ResourceUse::Signaling) { origins.insert(resource_origin(&resource.url).unwrap(), url); }`. A private scheme-filtered resource selector could keep both callers on the same configuration-policy implementation.

2. Possible **Duplicated Code**, `tests/irc_bootstrap.rs:213–223,341–351`: both new fixture clients repeat registration/channel-join handling. Quoted shared hunk: `if line.contains(" 001 ") { ... JOIN {CHANNEL} ... }` followed by `if line.contains(" 366 ") { break; }`. Extract the real fixture-client registration helper so later protocol handling fixes reach the observer and injection client together.

3. Possible **Data Clumps**, `src/irc_transport.rs:392–401,472–482`: `drive` and `read_loop` repeatedly pass the binding's route, service, address watcher, status and traffic separately. Quoted parameter group: `route: Route, service: Service, addresses: Watchable<Vec<CustomAddr>>, ... status: watch::Sender<Status>, traffic: Arc<Traffic>`. A small binding-state type could make ownership and lifecycle easier to follow while leaving read/write channels explicit.

Evidence review: authoritative `completed-acceptance` and `continued-short` receipts report `passed=true`; the earlier `final` receipt reports failure and is accurately explained in the evidence index. No tests were rerun and no runtime/test files were edited.

Summary: 0 hard violations; 3 low-severity maintainability observations, all judgement calls.

### Final incremental review

# MIOPUNCH-45 incremental Standards review

Reviewed cleanup diff `4c4b5dc387a56f9d968d2d49cf33cb098052131e...b4ec3c29fbf178fcb8b2564f22eededdb3820578` only. The original [Standards review](standards-review.md) is retained unchanged. Previously established repository standards and smell-baseline rules remain applicable.

All three original non-blocking observations are resolved:

- `src/resources.rs`: `mqtt_resources` and `irc_resources` now call the same private `signaling_resources(scheme)`. Parsing, permission filtering and origin deduplication have one implementation; carrier-specific entry points remain clear.
- `tests/irc_bootstrap.rs`: both fixture clients now use `registered_irc_client`. Registration and channel-join handling have one implementation, with bounded waiting and explicit failure if the real IRCd disconnects before registration completes.
- `src/irc_transport.rs`: `DriverBinding` gathers route, service, address watcher, status and traffic. `drive`, `read_loop` and `write_loop` borrow that owned binding; channels remain explicit. Driver completion still withdraws addresses, closes status and clears pending observations. Endpoint cancellation retains its existing owned-driver behavior.

No new documented-standard violation or actionable baseline smell was found in this cleanup. The private selector and binding type address demonstrated duplication and parameter grouping; they do not introduce unsupported runtime extension mechanisms or new product decisions.

Reviewed existing post-cleanup receipts: `review-refactor/results.json` reports `passed=true` for ten real IRC cases and the isolated UDP data-cut; `review-mqtt-filter/results.json` reports `passed=true` for configured MQTT startup; `review-compile/results.json` reports successful current-hook/default-native compilation and formatting. No checks were rerun, no slow NAT matrix was repeated, and no runtime/test files were edited by this reviewer.

Summary: 3 original observations resolved; 0 remaining Standards findings; 0 new Standards findings.

## Spec

# MIOPUNCH-45 independent Spec review

Reviewed committed diff `2b3711ab5a9af39a560902f52bcf8f6f46cd65b9...4c4b5dc387a56f9d968d2d49cf33cb098052131e` (one commit). Sources: live Plane MIOPUNCH-45, parent MIOPUNCH-14 and its approved Testing Decisions, CONTEXT, accepted ADRs 0004/0005. Existing user WIP and `.cargo/config.toml` are outside this review.

**Findings: 0.**

- **Missing/partial requirements:** none identified within the declared qualification. “将同一连接入口从 MQTT 换成真实 IRC 服务并完成引导” is covered by configured native binding, the shared bootstrap hook, real membership/TLS, and matching bootstrap/data stable IDs in `completed-acceptance/first.log`. Long actual QUIC handshakes, missing/repeated fragments, bounded reassembly, actual server fakelag, fresh binding and cancellation have separate acceptance cases in `tests/irc_bootstrap.rs`.
- **Unrequested scope creep:** none identified. The carrier, integration, resource filtering, runnable fixture and evidence support this ticket. No UI, KCP, service ACL or general runtime plugin platform is added.
- **Implemented-looking but wrong behavior:** none identified. “业务不在 IRC 上传输” and “旁听/注入负例及数据路径失效后的用途限制与 MQTT 分支一致” retain the shared admission/data-release boundary. Independent IRC observation, wrong-target/unapproved-member rejection, and real isolated IPv4/IPv6 UDP loss substantiate the tested boundary. `completed-acceptance/data-cut.log` preserves the original stable ID and records unchanged IRC publication counts after queued business fails.

`completed-acceptance/results.json` and `continued-short/results.json` report success; the earlier failed whole short-suite receipt is explicitly retained as failed. This review inspected code and existing execution evidence without rerunning tests. Qualification remains Linux loopback, real pinned Ergo with the documented service settings, and UDP data release. Arbitrary public IRC services, server TLS, transparent session reconnect, broader NAT/platform support and a cryptographic audit are not claimed or inferred. Fresh reconnect is distinct from the original session, consistent with the documented lifecycle outcome.

### Final incremental review

# MIOPUNCH-45 incremental Spec review

Reviewed only `4c4b5dc387a56f9d968d2d49cf33cb098052131e...b4ec3c29fbf178fcb8b2564f22eededdb3820578`, retaining the original independent review in `spec-review.md`. The same live MIOPUNCH-45, parent specification and accepted ADR baseline applies.

**New findings: 0.** No missing requirement, unrequested scope expansion or incorrect implementation was introduced by this increment.

- `src/resources.rs`: private `signaling_resources` preserves the prior MQTT/IRC scheme selection, configured-resource precedence, origin deduplication and `Signaling` permission filtering. This remains consistent with “替换 MQTT 为 IRC 不改变节点身份、成员判断及上层连接调用”.
- `src/irc_transport.rs`: private `DriverBinding` groups existing state and borrows it through the same owned driver. Routes, service tags, readiness, closure cleanup, wire envelopes, pacing, queues and reassembly bounds retain their previous behavior, satisfying the existing bounded lifecycle requirement.
- `tests/irc_bootstrap.rs`: shared `registered_irc_client` still uses real TCP registration and channel join. It adds an explicit eight-second fixture timeout and rejects early EOF; observation, injection and bounded-reassembly assertions remain intact.

Inspected the successful `review-refactor/results.json` receipt covering the ten IRC cases and real isolated UDP cut, `review-mqtt-filter/results.json` covering actual configured MQTT startup, and `review-compile/results.json` covering current hooks, default native compilation and formatting. No tests were rerun for this review. Qualification remains exactly the original declared Linux/Ergo/UDP environment; no broader service, NAT or platform claim is inferred.

Standards: initial 0 hard violations / 3 non-blocking maintenance suggestions, all accepted and resolved; final 0 open findings. Spec: initial and final 0 findings.
