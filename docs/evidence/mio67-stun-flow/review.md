# Independent code review

Initial range: `7f953ecdbf43226e994b6320efd97f2d5b48696e...c5a7c7ba88e0efe6f841c802bbdb3c2a35471f1e`.
Correction: `c5a7c7ba88e0efe6f841c802bbdb3c2a35471f1e...3aff833e2b5e8c7becde33c96fe5ab4dce922a5d`.
Unrelated unstaged user work is excluded.

## Standards

Reviewed `git diff 7f953ecdbf43226e994b6320efd97f2d5b48696e...c5a7c7ba88e0efe6f841c802bbdb3c2a35471f1e`; sole commit: `c5a7c7ba MIOPUNCH-67 Integrate QAD-first UDP STUN fallback and domain TCP punching`.

**Documented-standard violations: 0. Actual bugs identified: 0.**

Sources: root `AGENTS.md`, `/home/js/.codex/RTK.md`, `docs/agents/{domain,issue-tracker,triage-labels}.md`, current `CONTEXT.md`/design vocabulary, ADR 0005, and accepted `docs/notes/2026-10-03-address-observation-policy.md`. No separate coding-standard file or nested applicable AGENTS.md was found. The twelve Fowler smell heuristics were applied; tooling-enforced matters were excluded.

The diff preserves the official QAD list, records the observation-source/default-resource decision explicitly, separates Observation from DataRelay/Signaling, and retains TCP as explicitly enabled. The two intake notes distinguish accepted policy from fake-TCP research and defer work state to Plane. Evidence qualifies controlled Linux NAT success and Android TCP Binding separately; it does not claim public UDP success or Android TCP punching. No archived Go/OpenSpec gate was applied.

**Parked judgement call — possible Duplicated Code (1).** `scripts/tcp_punch_lab.py:83–92` repeats the side selection and four UDP NAT/filter commands already present at `74–81`, including:

> `iptables -t nat -I POSTROUTING 1 -o wan0 -s "$private" -p udp --sport "$port" -j SNAT --to-source "$public:$port"`

A shared shell helper or combined case condition could avoid maintaining the same fixture rules twice. This is an optional maintainability observation, not a documented-standard breach or demonstrated behavioral defect; leave it parked under the user's completion-first instruction.

Reviewed the fixed commit only; unrelated unstaged baseline/docs/cache work is excluded. No tests were rerun, no repository/index/tracker changes were made, and no refactor ticket was created.

Counts: **0 hard violations; 1 parked heuristic; 0 actual bugs.**

## Spec

Reviewed `git diff 7f953ecdbf43226e994b6320efd97f2d5b48696e...c5a7c7ba88e0efe6f841c802bbdb3c2a35471f1e` against the full live child/parent descriptions and accepted address-observation policy. Runtime verified by root as gpt-6.1-sol / high / priority.

**One concrete finding (P2): shared TCP supersession leaves a diagnostic socket blocking formal observation.**

Spec: “资源配置、真实观察及候选失效贯穿普通连接流程” and “取消/失败有界，没有假业务放行、信令业务回退或残留观察/打洞 socket。”

Trigger: on an explicitly enabled Linux IPv4 configured Endpoint, complete `ResourceEndpoint::observe_tcp_stun` successfully, retain that ResourceEndpoint, then start its ordinary message-bootstrap connection using the same sole permitted literal/domain STUN destination. `src/tcp_observation.rs:44` calls `TcpTransport::begin_observation`, whose new implementation (`src/tcp_transport.rs:133`) cancels only the previous token. The diagnostic's established stream remains strongly owned by `TcpObservationSlot::_stream` (`src/resources.rs:506`, assigned at line 636); only another diagnostic call removes that slot. Formal observation then opens a second connection from the same listener address to the same resolved IP:port (`src/tcp_observation.rs:72`; `src/tcp_transport.rs:683`). Linux cannot establish two connections with that identical four-tuple, so the observer becomes unavailable despite being healthy. With UDP/relay disabled, the formal connection fails; further formal retries still encounter the retained socket.

A narrow owned Linux socket check using the implementation's SO_REUSEADDR/SO_REUSEPORT and listener-port bind reproduced errno 99, “Cannot assign requested address”; all check sockets were closed. The existing pending-query test likewise acknowledges same-four-tuple reuse failure, but does not cover a completed diagnostic followed by formal punching. Fix ownership so supersession retires the retained diagnostic socket before reconnecting, or safely reuses a retained observation transaction/socket. Parent owns implementation and affected regression testing.

No additional missing requirements or unauthorized scope expansion found. Success, disable/restore, actual multi-destination/DNS-coalescence, same-Connection sync/business, platform boundaries and public-result qualifications are supported by the supplied evidence. No unchanged slow NAT matrix rerun; no architecture/refactor follow-up proposed.

### Spec closure

Reviewed `git diff c5a7c7ba88e0efe6f841c802bbdb3c2a35471f1e...3aff833e2b5e8c7becde33c96fe5ab4dce922a5d` against the approved child/parent requirements, accepted address-observation policy and initial Spec report. Root independently verified this reviewer as **gpt-6.1-sol / high / priority**; receipt: `/tmp/miopunch-priority-review_67_spec_closure_sol61_high.json`.

**Resolved: 1 P2. Open: 0. New findings: 0.**

Relevant requirements: “资源配置、真实观察及候选失效贯穿普通连接流程”; “失效、取消、网络变化和并发 supersession 不复活旧候选”; “取消/失败有界，没有假业务放行、信令业务回退或残留观察/打洞 socket。”

The original trigger was a successful retained diagnostic followed by ordinary formal TCP observation using the same listener/destination four-tuple. `TcpTransport::begin_observation` (`src/tcp_transport.rs:154`) now replaces a carrier-owned slot containing both the cancellation token and strong diagnostic stream. Dropping the old slot cancels its request and releases its stream before the next connection. `ResourceEndpoint` no longer owns a separate strong-stream map; conditional retention rejects superseded requests. Carrier cleanup and drop clear the shared slots (`src/tcp_transport.rs:451`, `:463`), also closing diagnostics when the raw Endpoint closes while its wrapper remains alive. Public observations retain weak stream references and existing endpoint/generation/token freshness checks. Formal observation keeps its own socket through validation and bilateral commit as before.

The saved actual RED reproduces errno99 at both kernel-NAT nodes. `review-fix/formal/` shows the same 42027/43027 source ports completing fresh Binding transactions, invalidating old diagnostics, validating TCP paths on the original TLS Connection, saving a new record 2 → 3 and exchanging authorized business. The raw-close public regression verifies peer EOF; affected broker-backed resources/TCP tests report 22 passes. Native checks and rebuilt physical Android observation disable/restore receipts support the changed lifecycle.

No new missing/partial requirement, scope expansion or incorrect behavior found in the correction. The retained UDP/QAD evidence remains applicable; the unchanged slow NAT matrix does not require repetition. This closure required no additional test gate or repository/tracker changes.

Counts: Standards 0 hard violations / 1 parked heuristic; Spec 1 P2 found and resolved / 0 open / 0 new.

All three independent review turns used gpt-6.1-sol / high / priority, verified by root against actual runtime settings. The requested optional architecture work remains parked.
